//! Editing `.stew` files: `stew add` and `stew remove`.
//!
//! Both commands describe what they are about to do and ask for
//! confirmation, since creating or emptying a `.stew` file changes who
//! owns a whole subtree, not just the directory named.

use crate::model::{self, StewMap, Steward};
use anyhow::{bail, Context, Result};
use clap::Args;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Args, Debug, Default)]
pub struct Opts {
    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub yes: bool,
    /// Suppress output after the change is made
    #[arg(short, long)]
    pub quiet: bool,
}

/// The positional arguments of `add`/`remove`: either `MEMBER DIR`, or
/// `me DIR` to use the member whose identities include the one from git
/// config.
#[derive(Args, Debug)]
pub struct Target {
    /// Member key and directory (or `me` and a directory)
    #[arg(required = true, num_args = 2, value_names = ["STEWARD", "DIR"], hide_possible_values = true)]
    pub args: Vec<String>,
}

impl Target {
    pub fn resolve(&self, map: &StewMap) -> Result<(PathBuf, String)> {
        let [a, b] = self.args.as_slice() else {
            unreachable!("clap enforces exactly two arguments");
        };
        if a == "me" {
            let id = git_identity(&map.root)?;
            let Some(key) = map.manifest.member_for_identity(&id) else {
                bail!(
                    "no member in {} has the identity {id} (from git config)",
                    model::ROOT_MANIFEST
                );
            };
            Ok((PathBuf::from(b), key.to_string()))
        } else {
            Ok((PathBuf::from(b), a.clone()))
        }
    }
}

/// The committer identity from git config, as `Name <email>`, read from
/// `root` so repo-local config applies.
pub fn git_identity(root: &Path) -> Result<String> {
    let get = |key: &str| -> Result<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["config", "--get", key])
            .output()
            .context("cannot run git")?;
        let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || val.is_empty() {
            bail!("git config {key} is not set; cannot determine who \"me\" is");
        }
        Ok(val)
    };
    Ok(format!("{} <{}>", get("user.name")?, get("user.email")?))
}

/// Human-readable label for a root-relative directory.
fn label(rel: &Path) -> String {
    if rel.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel.display().to_string()
    }
}

/// Root-relative path of the `.stew` file for `rel`, as shown to the user.
fn stew_label(rel: &Path) -> String {
    rel.join(model::STEW_FILE).display().to_string()
}

fn print_list(stewards: &[Steward]) {
    for s in stewards {
        println!("- {s}");
    }
}

/// Ask for confirmation unless `yes` was given. Declining exits with status 1.
fn confirm(opts: &Opts) -> Result<()> {
    if opts.yes {
        return Ok(());
    }
    print!("Proceed? [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    if matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        eprintln!("aborted");
        std::process::exit(1);
    }
}

fn read_stew(path: &Path) -> Result<Option<String>> {
    if path.is_file() {
        let text =
            fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        Ok(Some(text))
    } else {
        Ok(None)
    }
}

/// The stewards a directory inherits when it has no `.stew` of its own:
/// those of the governing `.stew` above, with that file's label for the
/// prompts. `None` if nothing governs it.
fn inherited<'a>(map: &'a StewMap, rel: &Path) -> Option<(String, &'a [Steward])> {
    let owner = map.governing_dir(rel)?;
    Some((stew_label(owner), &map.dirs[owner]))
}

/// Write `stewards` as a fresh `.stew` file.
fn write_new(file: &Path, stewards: &[&str]) -> Result<()> {
    let mut out = String::new();
    for s in stewards {
        out.push_str(s);
        out.push('\n');
    }
    fs::write(file, out).with_context(|| format!("cannot write {}", file.display()))
}

/// Make `steward` a steward of `dir`. If `dir` has its own `.stew`, the
/// steward is appended to it; otherwise a `.stew` is created naming the
/// stewards `dir` currently inherits plus the new one, so nobody loses
/// anything.
pub fn add(map: &StewMap, dir: &Path, steward: &str, opts: &Opts) -> Result<()> {
    let steward = steward.trim();
    if !map.manifest.members.contains_key(steward) {
        bail!(
            "{steward:?} is not a member; add it to [members] in {} first",
            model::ROOT_MANIFEST
        );
    }
    let rel = map.resolve_dir(dir)?;
    let file = map.root.join(&rel).join(model::STEW_FILE);
    let shown = stew_label(&rel);

    if let Some(text) = read_stew(&file)? {
        let current = model::parse_stew(&text);
        if current.iter().any(|s| s == steward) {
            if !opts.quiet {
                println!("{steward} is already a steward in {shown}");
            }
            return Ok(());
        }
        println!("* Add {steward} to {shown}, which currently names:");
        if current.is_empty() {
            println!("- (nobody: {} is explicitly unowned)", label(&rel));
        } else {
            print_list(&current);
        }
        println!();
        confirm(opts)?;

        let mut out = text;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(steward);
        out.push('\n');
        fs::write(&file, out).with_context(|| format!("cannot write {}", file.display()))?;
    } else {
        let mut stewards: Vec<&str> = Vec::new();
        match inherited(map, &rel) {
            Some((from, current)) if current.iter().any(|s| s == steward) => {
                if !opts.quiet {
                    println!(
                        "{steward} is already a steward of {} (via {from})",
                        label(&rel)
                    );
                }
                return Ok(());
            }
            Some((from, current)) if !current.is_empty() => {
                println!("* {} currently inherits its stewards from {from}:", label(&rel));
                print_list(current);
                println!("* Create {shown} naming those plus {steward}. ({from} itself is unchanged.)");
                stewards.extend(current.iter().map(String::as_str));
            }
            Some((from, _)) => println!(
                "* {} is currently explicitly unowned via {from}.\n* Create {shown} naming {steward}; it and everything below it will be {steward}'s.",
                label(&rel)
            ),
            None => println!(
                "* Nobody currently stewards {}.\n* Create {shown} naming {steward}; it and everything below it will be {steward}'s.",
                label(&rel)
            ),
        }
        stewards.push(steward);
        println!();
        confirm(opts)?;
        write_new(&file, &stewards)?;
    }
    if !opts.quiet {
        println!("added {steward} to {shown}");
    }
    Ok(())
}

/// Stop `steward` stewarding `dir`. If `dir` has its own `.stew`, the
/// steward's line is dropped from it; otherwise a `.stew` is created naming
/// the stewards `dir` currently inherits minus this one, so the levels
/// above are unaffected.
pub fn remove(map: &StewMap, dir: &Path, steward: &str, opts: &Opts) -> Result<()> {
    let steward = steward.trim();
    let rel = map.resolve_dir(dir)?;
    let file = map.root.join(&rel).join(model::STEW_FILE);
    let shown = stew_label(&rel);

    let existing = read_stew(&file)?;
    let (current, from): (Vec<Steward>, Option<String>) = match &existing {
        Some(text) => (model::parse_stew(text), None),
        None => match inherited(map, &rel) {
            Some((from, current)) => (current.to_vec(), Some(from)),
            None => (Vec::new(), None),
        },
    };
    if !current.iter().any(|s| s == steward) {
        if !opts.quiet {
            println!("{steward} is not a steward of {}", label(&rel));
        }
        return Ok(());
    }
    let remaining: Vec<&str> = current
        .iter()
        .map(String::as_str)
        .filter(|s| *s != steward)
        .collect();

    match &from {
        Some(from) => {
            println!("* {} currently inherits its stewards from {from}:", label(&rel));
            print_list(&current);
            println!("* Create {shown} naming those without {steward}. ({from} itself is unchanged.)");
        }
        None => println!("* Remove {steward} from {shown}."),
    }
    if remaining.is_empty() {
        println!(
            "* This leaves {shown} empty: nobody will steward {} or anything below it.",
            label(&rel)
        );
        let fallback = rel.parent().and_then(|p| map.governing_dir(p));
        match (from.is_some(), fallback) {
            (true, _) => {}
            (false, Some(owner)) => println!(
                "  (Delete the file instead to fall back to {}.)",
                stew_label(owner)
            ),
            (false, None) => println!("  (Deleting the file instead would have the same effect, since no parent .stew applies.)"),
        }
    } else {
        println!("* {shown} will then name:");
        for s in &remaining {
            println!("- {s}");
        }
    }
    println!();
    confirm(opts)?;

    match existing {
        Some(text) => {
            // Drop only the matching lines, preserving comments and blanks.
            let mut out: String = text
                .lines()
                .filter(|l| l.trim() != steward)
                .map(|l| format!("{l}\n"))
                .collect();
            if out.trim().is_empty() {
                out.clear();
            }
            fs::write(&file, out).with_context(|| format!("cannot write {}", file.display()))?;
        }
        None => write_new(&file, &remaining)?,
    }
    if !opts.quiet {
        println!("removed {steward} from {shown}");
    }
    Ok(())
}
