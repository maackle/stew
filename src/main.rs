mod diff;
mod edit;
mod model;

use anyhow::Result;
use clap::{Parser, Subcommand};
use model::{build_tree, Node, StewMap, Steward};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "stew",
    about = "Stewardship linter: calculations over the tree defined by .stew files"
)]
struct Cli {
    /// Project root (defaults to searching upward from the current
    /// directory for stew.toml).
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the tree of stewardship transitions
    Tree,
    /// Show lines owned per steward, and each steward's share of the total
    Stats,
    /// Read a unified diff from stdin and attribute it to stewards
    Diff,
    /// List every steward named in any .stew file, alphabetically
    Stewards,
    /// List the unclaimed subtrees at or below a directory
    ///
    /// A directory is unclaimed when no `.stew` governs it, or the governing
    /// `.stew` is empty. Only the top of each unclaimed subtree is listed.
    Unclaimed {
        /// Directory to start from
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Print the path of the .stew file governing a directory
    Which {
        /// Directory to look up
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Add a steward to DIR/.stew, creating the file if needed
    ///
    /// `stew add me DIR` adds the identity from git config
    /// (user.name <user.email>).
    #[command(
        override_usage = "stew add [OPTIONS] <STEWARD> <DIR>\n       stew add [OPTIONS] me <DIR>"
    )]
    Add {
        #[command(flatten)]
        target: edit::Target,
        #[command(flatten)]
        opts: edit::Opts,
    },
    /// Remove a steward from DIR/.stew (alias: rm)
    ///
    /// `stew remove me DIR` removes the identity from git config
    /// (user.name <user.email>).
    #[command(
        alias = "rm",
        override_usage = "stew remove [OPTIONS] <STEWARD> <DIR>\n       stew remove [OPTIONS] me <DIR>"
    )]
    Remove {
        #[command(flatten)]
        target: edit::Target,
        #[command(flatten)]
        opts: edit::Opts,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = match &cli.root {
        Some(r) => model::find_root(r)?,
        None => model::find_root(Path::new("."))?,
    };
    let map = StewMap::load(&root)?;

    match cli.command {
        Command::Tree => tree(&map),
        Command::Stats => stats(&map)?,
        Command::Diff => cmd_diff(&map)?,
        Command::Stewards => stewards(&map),
        Command::Unclaimed { dir } => unclaimed(&map, &dir)?,
        Command::Which { dir } => which(&map, &dir)?,
        Command::Add { target, opts } => {
            let (dir, steward) = target.resolve(&map.root)?;
            edit::add(&map, &dir, &steward, &opts)?
        }
        Command::Remove { target, opts } => {
            let (dir, steward) = target.resolve(&map.root)?;
            edit::remove(&map, &dir, &steward, &opts)?
        }
    }
    Ok(())
}

fn fmt_stewards(stewards: &[Steward]) -> String {
    if stewards.is_empty() {
        "(no steward)".to_string()
    } else {
        stewards.join(", ")
    }
}

// ---------------------------------------------------------------- tree ----

fn tree(map: &StewMap) {
    let root = build_tree(map);
    println!(". — {}", fmt_stewards(&root.stewards));
    print_children(&root, "");
}

fn print_children(node: &Node, indent: &str) {
    let n = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let last = i == n - 1;
        let branch = if last { "└── " } else { "├── " };
        let label = child
            .rel
            .strip_prefix(&node.rel)
            .unwrap_or(&child.rel)
            .display();
        println!(
            "{indent}{branch}{label} — {}",
            fmt_stewards(&child.stewards)
        );
        let next = format!("{indent}{}", if last { "    " } else { "│   " });
        print_children(child, &next);
    }
}

// --------------------------------------------------------------- stats ----

const UNOWNED: &str = "(UNCLAIMED)";

fn stats(map: &StewMap) -> Result<()> {
    let mut lines_by_steward: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut shared = false;

    for entry in model::walker(&map.root).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let name = entry.file_name();
        if name == model::STEW_FILE || name == model::ROOT_MANIFEST {
            continue;
        }
        let Some(lines) = count_lines(entry.path())? else {
            continue; // binary file
        };
        total += lines;
        let rel = entry.path().strip_prefix(&map.root).unwrap();
        match map.stewards_of_file(&rel.to_string_lossy()) {
            Some(stewards) if !stewards.is_empty() => {
                shared |= stewards.len() > 1;
                for s in stewards {
                    *lines_by_steward.entry(s.clone()).or_default() += lines;
                }
            }
            // No governing .stew, or an empty one: both mean unowned.
            _ => *lines_by_steward.entry(UNOWNED.to_string()).or_default() += lines,
        }
    }

    let mut rows: Vec<(&String, &usize)> = lines_by_steward.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));

    let width = rows
        .iter()
        .map(|(s, _)| s.len())
        .chain(["total".len()])
        .max()
        .unwrap_or(5);
    for (steward, lines) in &rows {
        let pct = if total == 0 {
            0.0
        } else {
            100.0 * **lines as f64 / total as f64
        };
        println!("{steward:<width$}  {lines:>8}  {pct:>5.1}%");
    }
    println!("{:<width$}  {total:>8}", "total");
    if shared {
        println!("note: some directories have multiple stewards; shared lines count fully for each, so percentages can sum to more than 100%");
    }
    Ok(())
}

/// Line count of a text file; `None` for binary files (NUL byte in the
/// first 8 KiB).
fn count_lines(path: &Path) -> Result<Option<usize>> {
    let bytes = fs::read(path)?;
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.contains(&0) {
        return Ok(None);
    }
    // `wc -l` plus a trailing partial line, if any.
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let partial = usize::from(!bytes.is_empty() && *bytes.last().unwrap() != b'\n');
    Ok(Some(newlines + partial))
}

// ------------------------------------------------------------ stewards ----

/// Print every distinct steward named in any `.stew` file, one per line,
/// sorted alphabetically.
fn stewards(map: &StewMap) {
    let all: BTreeSet<&Steward> = map.dirs.values().flatten().collect();
    for s in all {
        println!("{s}");
    }
}

// ----------------------------------------------------------- unclaimed ----

/// A directory is an unclaimed subtree when nothing in it is claimed: no
/// `.stew` governs it (or the governing one names nobody), and no `.stew`
/// at or below it names anybody.
fn is_unclaimed_subtree(map: &StewMap, rel: &Path) -> bool {
    let governed = map
        .governing_dir(rel)
        .is_some_and(|d| !map.dirs[d].is_empty());
    let claimed_below = map
        .dirs
        .iter()
        .any(|(d, stewards)| d.starts_with(rel) && !stewards.is_empty());
    !governed && !claimed_below
}

/// Print every unclaimed subtree at or below `dir`, as paths relative to
/// the project root, one per line. Only the top of each unclaimed subtree
/// is listed; a directory inside one is not listed again.
fn unclaimed(map: &StewMap, dir: &Path) -> Result<()> {
    let start = map.resolve_dir(dir)?;
    for entry in model::walker(&map.root.join(&start)).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_dir()) {
            continue;
        }
        let rel = entry.path().strip_prefix(&map.root).unwrap();
        if !is_unclaimed_subtree(map, rel) {
            continue;
        }
        let parent_listed = rel != start
            && rel
                .parent()
                .is_some_and(|p| is_unclaimed_subtree(map, p));
        if !parent_listed {
            let shown = if rel.as_os_str().is_empty() { "." } else { rel.to_str().unwrap_or(".") };
            println!("{shown}");
        }
    }
    Ok(())
}

// --------------------------------------------------------------- which ----

/// Print the absolute path of the governing .stew file to stdout, so it can
/// be piped (e.g. `cat $(stew which)`). If none applies, print a message to
/// stderr and exit non-zero, keeping stdout clean.
fn which(map: &StewMap, dir: &Path) -> Result<()> {
    let rel = map.resolve_dir(dir)?;
    match map.governing_dir(&rel) {
        Some(owner) => {
            println!("{}", map.root.join(owner).join(model::STEW_FILE).display());
            Ok(())
        }
        None => {
            eprintln!("no .stew file at this level");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------- diff ----

fn cmd_diff(map: &StewMap) -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let files = diff::parse(&input);
    if files.is_empty() {
        println!("no file changes found in diff");
        return Ok(());
    }

    // Group files under the steward set that owns them.
    let mut groups: BTreeMap<String, Vec<&diff::FileDiff>> = BTreeMap::new();
    for f in &files {
        let key = match map.stewards_of_file(&f.path) {
            Some(s) if !s.is_empty() => fmt_stewards(s),
            _ => UNOWNED.to_string(),
        };
        groups.entry(key).or_default().push(f);
    }

    let mut first = true;
    for (stewards, files) in &groups {
        if !first {
            println!();
        }
        first = false;
        let (a, r) = files
            .iter()
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        println!("{stewards}  (+{a} -{r})");
        for f in files {
            println!("  {}  +{} -{}", f.path, f.added, f.removed);
        }
    }
    Ok(())
}
