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
    /// Check that every .stew file names only members listed in stew.toml
    ///
    /// Every other command runs this check first and refuses to proceed if
    /// it fails.
    Check,
    /// Show the tree of stewardship transitions
    Tree,
    /// Show lines owned per steward, and each steward's share of the total.
    ///
    /// Files ignored by .gitignore or .stewignore are not counted.
    ///
    /// Note that shared lines count fully for each steward listed, so percentages can sum to more than 100%.
    ///
    /// An OVERLAP table follows, showing how many lines are covered by 0, 1, 2, ... stewards.
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
    /// Make a member a steward of DIR
    ///
    /// Appends to DIR/.stew, or creates it naming the stewards DIR
    /// currently inherits plus the new one. `stew add me DIR` uses the
    /// member matching the identity from git config.
    #[command(
        override_usage = "stew add [OPTIONS] <STEWARD> <DIR>\n       stew add [OPTIONS] me <DIR>"
    )]
    Add {
        #[command(flatten)]
        target: edit::Target,
        #[command(flatten)]
        opts: edit::Opts,
    },
    /// Stop a member stewarding DIR (alias: rm)
    ///
    /// Edits DIR/.stew, or creates it naming the stewards DIR currently
    /// inherits minus this one. `stew remove me DIR` uses the member
    /// matching the identity from git config.
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
    if !check(&map) {
        std::process::exit(1);
    }

    match cli.command {
        Command::Check => {}
        Command::Tree => tree(&map),
        Command::Stats => stats(&map)?,
        Command::Diff => cmd_diff(&map)?,
        Command::Stewards => stewards(&map),
        Command::Unclaimed { dir } => unclaimed(&map, &dir)?,
        Command::Which { dir } => which(&map, &dir)?,
        Command::Add { target, opts } => {
            let (dir, steward) = target.resolve(&map)?;
            edit::add(&map, &dir, &steward, &opts)?
        }
        Command::Remove { target, opts } => {
            let (dir, steward) = target.resolve(&map)?;
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

// --------------------------------------------------------------- check ----

/// Report every key in a `.stew` file that is not a member in `stew.toml`,
/// one per line on stderr. Returns whether the project is consistent.
fn check(map: &StewMap) -> bool {
    let unknown = map.unknown_members();
    for (dir, key) in &unknown {
        eprintln!(
            "{}: unknown member {key:?} (not in [members] of {})",
            dir.join(model::STEW_FILE).display(),
            model::ROOT_MANIFEST
        );
    }
    unknown.is_empty()
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

const UNOWNED: &str = "(unclaimed)";

fn stats(map: &StewMap) -> Result<()> {
    let mut lines_by_steward: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    // Lines keyed by how many stewards cover them (0 = unclaimed).
    let mut lines_by_count: BTreeMap<usize, usize> = BTreeMap::new();

    for entry in model::walker(&map.root).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(lines) = count_lines(entry.path())? else {
            continue; // binary file
        };
        total += lines;
        let rel = entry.path().strip_prefix(&map.root).unwrap();
        match map.stewards_of_file(&rel.to_string_lossy()) {
            Some(stewards) if !stewards.is_empty() => {
                *lines_by_count.entry(stewards.len()).or_default() += lines;
                for s in stewards {
                    *lines_by_steward.entry(s).or_default() += lines;
                }
            }
            // No governing .stew, or an empty one: both mean unowned.
            _ => {
                *lines_by_count.entry(0).or_default() += lines;
                *lines_by_steward.entry(UNOWNED.to_string()).or_default() += lines;
            }
        }
    }

    let pct = |lines: usize| {
        if total == 0 {
            0.0
        } else {
            100.0 * lines as f64 / total as f64
        }
    };

    let mut rows: Vec<(&String, &usize)> = lines_by_steward.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));

    // Column width fits the longest name and both tables' headings.
    let width = rows
        .iter()
        .map(|(s, _)| s.len())
        .chain(["STEWARD".len(), "OVERLAP".len()])
        .max()
        .unwrap();

    println!("total lines: {total}");
    println!();
    println!("{:<width$}  {:>8}  {:>6}", "STEWARD", "LINES", "PERCENT");
    for (steward, lines) in &rows {
        println!("{steward:<width$}  {lines:>8}  {:>5.1}%", pct(**lines));
    }

    // Overlap: how much of the code is covered by 0, 1, 2, ... stewards.
    // Every count from 0 up to the maximum is listed, so gaps show as 0.
    println!();
    println!("{:<width$}  {:>8}  {:>6}", "OVERLAP", "LINES", "PERCENT");
    let max_count = lines_by_count.keys().last().copied().unwrap_or(0);
    for n in 0..=max_count {
        let lines = lines_by_count.get(&n).copied().unwrap_or(0);
        println!("{n:<width$}  {lines:>8}  {:>5.1}%", pct(lines));
    }
    Ok(())
}

/// Line count of a text file; `None` for binary files (NUL byte in the
/// first 8 KiB).
/// Total lines of all text files under `dir`, with the same rules as
/// `stats`: ignore files respected, binary files skipped, and files
/// claimed by a member's override not counted.
fn count_lines_under(map: &StewMap, dir: &Path) -> Result<usize> {
    let mut total = 0;
    for entry in model::walker(dir).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let rel = entry.path().strip_prefix(&map.root).unwrap();
        if map.is_overridden(&rel.to_string_lossy()) {
            continue;
        }
        if let Some(lines) = count_lines(entry.path())? {
            total += lines;
        }
    }
    Ok(total)
}

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
/// the project root with the total lines under each, one per line. Only the
/// top of each unclaimed subtree is listed; a directory inside one is not
/// listed again.
fn unclaimed(map: &StewMap, dir: &Path) -> Result<()> {
    let start = map.resolve_dir(dir)?;
    let mut rows: Vec<(String, usize)> = Vec::new();
    for entry in model::walker(&map.root.join(&start)).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_dir()) {
            continue;
        }
        let rel = entry.path().strip_prefix(&map.root).unwrap();
        if !is_unclaimed_subtree(map, rel) {
            continue;
        }
        let parent_listed =
            rel != start && rel.parent().is_some_and(|p| is_unclaimed_subtree(map, p));
        if !parent_listed {
            let shown = if rel.as_os_str().is_empty() {
                "."
            } else {
                rel.to_str().unwrap_or(".")
            };
            rows.push((shown.to_string(), count_lines_under(map, entry.path())?));
        }
    }
    let width = rows.iter().map(|(p, _)| p.len()).max().unwrap_or(0);
    for (path, lines) in &rows {
        println!("{path:<width$}  {lines:>8}");
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
            Some(s) if !s.is_empty() => fmt_stewards(&s),
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
