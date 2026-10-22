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

    let existing = read_stew(&file)?;
    match &existing {
        Some(text) => {
            let current = model::parse_stew(text);
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
        }
        None => {
            println!("* Create {shown} naming {steward}.");
            match map.governing_dir(&rel) {
                Some(owner) => {
                    let owner_stewards = &map.dirs[owner];
                    if owner_stewards.is_empty() {
                        println!(
                            "* {} is currently explicitly unowned via {}; it and everything below it will become {steward}'s.",
                            label(&rel),
                            stew_label(owner)
                        );
                    } else {
                        println!(
                            "* This takes stewardship of {} (and everything below it) away from {}, which names:",
                            label(&rel),
                            stew_label(owner)
                        );
                        print_list(owner_stewards);
                    }
                }
                None => println!(
                    "* Nobody currently stewards {}; it and everything below it will be stewarded by {steward}.",
                    label(&rel)
                ),
            }
        }
    }
    println!();
    confirm(opts)?;

    let mut out = existing.unwrap_or_default();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(steward);
    out.push('\n');
    fs::write(&file, out).with_context(|| format!("cannot write {}", file.display()))?;
    if !opts.quiet {
        println!("added {steward} to {shown}");
    }
    Ok(())
}

pub fn remove(map: &StewMap, dir: &Path, steward: &str, opts: &Opts) -> Result<()> {
    let steward = steward.trim();
    let rel = map.resolve_dir(dir)?;
    let file = map.root.join(&rel).join(model::STEW_FILE);
    let shown = stew_label(&rel);

    let Some(text) = read_stew(&file)? else {
        match map.governing_dir(&rel) {
            Some(owner) => bail!(
                "no {shown}; {} is governed by {} (use `stew remove {} ...` to edit that)",
                label(&rel),
                stew_label(owner),
                label(owner)
            ),
            None => bail!("no {shown}, and no .stew file governs {}", label(&rel)),
        }
    };
    let current = model::parse_stew(&text);
    if !current.iter().any(|s| s == steward) {
        if !opts.quiet {
            println!("{steward} is not a steward in {shown}");
        }
        return Ok(());
    }
    let remaining: Vec<&Steward> = current.iter().filter(|s| *s != steward).collect();

    println!("* Remove {steward} from {shown}.");
    if remaining.is_empty() {
        println!(
            "* This leaves {shown} empty: nobody will steward {} or anything below it.",
            label(&rel)
        );
        let fallback = rel.parent().and_then(|p| map.governing_dir(p));
        match fallback {
            Some(owner) => println!(
                "  (Delete the file instead to fall back to {}.)",
                stew_label(owner)
            ),
            None => println!("  (Deleting the file instead would have the same effect, since no parent .stew applies.)"),
        }
    } else {
        println!("* {shown} will then name:");
        for s in &remaining {
            println!("- {s}");
        }
    }
    println!();
    confirm(opts)?;

    // Drop only the matching lines, preserving comments and blank lines.
    let mut out: String = text
        .lines()
        .filter(|l| l.trim() != steward)
        .map(|l| format!("{l}\n"))
        .collect();
    if out.trim().is_empty() {
        out.clear();
    }
    fs::write(&file, out).with_context(|| format!("cannot write {}", file.display()))?;
    if !opts.quiet {
        println!("removed {steward} from {shown}");
    }
    Ok(())
}
