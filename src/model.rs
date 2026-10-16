//! The stewardship model: locating the project root, parsing `.stew` files,
//! and building the tree of stewardship transitions.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const ROOT_MANIFEST: &str = "stew.toml";
pub const STEW_FILE: &str = ".stew";
pub const STEW_IGNORE: &str = ".stewignore";

/// A steward identity as it appears in a `.stew` file, e.g.
/// `Jane Doe <jane@example.com>`. Kept as the raw (trimmed) line; identities
/// compare equal on their exact text.
pub type Steward = String;

/// Walk up from `start` looking for a directory containing `stew.toml`.
pub fn find_root(start: &Path) -> Result<PathBuf> {
    let start = start
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", start.display()))?;
    for dir in start.ancestors() {
        if dir.join(ROOT_MANIFEST).is_file() {
            return Ok(dir.to_path_buf());
        }
    }
    bail!(
        "no {} found in {} or any parent directory",
        ROOT_MANIFEST,
        start.display()
    );
}

/// Validate the root manifest. It is currently a placeholder, but it must at
/// least be well-formed TOML so we can add fields later without breaking
/// existing projects.
pub fn load_manifest(root: &Path) -> Result<()> {
    let path = root.join(ROOT_MANIFEST);
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    text.parse::<toml::Table>()
        .with_context(|| format!("{} is not valid TOML", path.display()))?;
    Ok(())
}

/// Parse a `.stew` file: one steward per line; blank lines and `#` comments
/// are ignored.
pub fn parse_stew(text: &str) -> Vec<Steward> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The full stewardship map of a project: every directory (relative to the
/// root) that contains a `.stew` file, with its stewards. Directories are
/// keyed by their relative path (the root itself is keyed by `""`). An empty
/// steward list means the directory has a `.stew` file that names nobody:
/// the subtree is explicitly unowned, rather than inheriting from above.
#[derive(Debug, Default)]
pub struct StewMap {
    pub root: PathBuf,
    pub dirs: BTreeMap<PathBuf, Vec<Steward>>,
}

impl StewMap {
    /// Scan the project (respecting `.gitignore` etc.) and collect all
    /// `.stew` files.
    pub fn load(root: &Path) -> Result<Self> {
        load_manifest(root)?;
        let mut dirs = BTreeMap::new();
        for entry in walker(root).build() {
            let entry = entry?;
            if entry.file_type().is_some_and(|t| t.is_file())
                && entry.file_name() == STEW_FILE
            {
                let text = fs::read_to_string(entry.path())
                    .with_context(|| format!("cannot read {}", entry.path().display()))?;
                // An empty `.stew` is meaningful: it says nobody stewards
                // this directory or anything below it.
                let stewards = parse_stew(&text);
                let dir = entry
                    .path()
                    .parent()
                    .unwrap()
                    .strip_prefix(root)
                    .unwrap()
                    .to_path_buf();
                dirs.insert(dir, stewards);
            }
        }
        Ok(StewMap {
            root: root.to_path_buf(),
            dirs,
        })
    }

    /// Resolve a user-supplied directory to a path relative to the project
    /// root, checking that it exists and lies inside the project.
    pub fn resolve_dir(&self, dir: &Path) -> Result<PathBuf> {
        let abs = dir
            .canonicalize()
            .with_context(|| format!("cannot resolve {}", dir.display()))?;
        if !abs.is_dir() {
            bail!("{} is not a directory", dir.display());
        }
        let rel = abs.strip_prefix(&self.root).map_err(|_| {
            anyhow::anyhow!(
                "{} is outside the project root {}",
                dir.display(),
                self.root.display()
            )
        })?;
        Ok(rel.to_path_buf())
    }

    /// The directory (relative to the root) whose `.stew` governs `rel`:
    /// the nearest directory at or above `rel` with a `.stew` file. `None`
    /// if no `.stew` governs it.
    pub fn governing_dir(&self, rel: &Path) -> Option<&Path> {
        let mut dir = Some(rel);
        while let Some(d) = dir {
            if let Some((k, _)) = self.dirs.get_key_value(d) {
                return Some(k);
            }
            dir = if d.as_os_str().is_empty() {
                None
            } else {
                d.parent().or(Some(Path::new("")))
            };
        }
        None
    }

    /// The stewards owning a file path (relative to the root): those named
    /// by the nearest `.stew` at or above its directory. `None` if no
    /// `.stew` governs it.
    pub fn stewards_of_file(&self, rel: &str) -> Option<&[Steward]> {
        let p = Path::new(rel);
        let parent = p.parent().unwrap_or(Path::new(""));
        let mut dir = Some(parent);
        while let Some(d) = dir {
            if let Some(s) = self.dirs.get(d) {
                return Some(s);
            }
            dir = if d.as_os_str().is_empty() {
                None
            } else {
                d.parent().or(Some(Path::new("")))
            };
        }
        None
    }
}

/// A node in the stewardship tree: a directory where stewardship changes
/// hands, labelled with its path relative to the parent node.
#[derive(Debug)]
pub struct Node {
    /// Path relative to the project root ("" for the root node).
    pub rel: PathBuf,
    pub stewards: Vec<Steward>,
    pub children: Vec<Node>,
}

/// Build the tree of stewardship transitions. The root node is always
/// present; every other `.stew` directory is nested under the nearest
/// ancestor that also has one (or the root).
pub fn build_tree(map: &StewMap) -> Node {
    let mut root = Node {
        rel: PathBuf::new(),
        stewards: map.dirs.get(Path::new("")).cloned().unwrap_or_default(),
        children: Vec::new(),
    };
    // BTreeMap iteration is lexicographic, so parents come before children.
    for (dir, stewards) in &map.dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        insert(&mut root, dir, stewards);
    }
    root
}

fn insert(node: &mut Node, dir: &Path, stewards: &[Steward]) {
    // Find a child whose subtree contains dir.
    for child in &mut node.children {
        if dir.starts_with(&child.rel) {
            insert(child, dir, stewards);
            return;
        }
    }
    node.children.push(Node {
        rel: dir.to_path_buf(),
        stewards: stewards.to_vec(),
        children: Vec::new(),
    });
}

/// Which ignore files a walk honours. `.git` is always excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ignores {
    /// Nothing but `.git`.
    None,
    /// `.gitignore` (and git's global/exclude files).
    Git,
    /// `.gitignore` plus `.stewignore`, the normal mode for every command.
    GitAndStew,
}

/// A walker rooted at `root` honouring the given ignore files. `.stew` files
/// are hidden files, so hidden entries must be included; `.git` is excluded
/// explicitly. `.stewignore` uses gitignore syntax and may appear in any
/// directory, applying to that directory's subtree.
pub fn walker_with(root: &Path, ignores: Ignores) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(false).filter_entry(|e| e.file_name() != ".git");
    let git = ignores != Ignores::None;
    b.git_ignore(git).git_global(git).git_exclude(git);
    if ignores == Ignores::GitAndStew {
        b.add_custom_ignore_filename(STEW_IGNORE);
    }
    b
}

/// The normal walker: respects `.gitignore` and `.stewignore`.
pub fn walker(root: &Path) -> ignore::WalkBuilder {
    walker_with(root, Ignores::GitAndStew)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_comments_and_blanks() {
        let s = parse_stew("# team\nAlice <a@x.com>\n\n  Bob <b@x.com>  \n");
        assert_eq!(s, vec!["Alice <a@x.com>", "Bob <b@x.com>"]);
    }

    #[test]
    fn nearest_stew_wins() {
        let mut map = StewMap::default();
        map.dirs
            .insert(PathBuf::from(""), vec!["Root <r@x>".into()]);
        map.dirs
            .insert(PathBuf::from("x"), vec!["A <a@x>".into()]);
        map.dirs
            .insert(PathBuf::from("x/y/z"), vec!["B <b@x>".into()]);

        assert_eq!(map.stewards_of_file("x/y/z/f.rs").unwrap()[0], "B <b@x>");
        assert_eq!(map.stewards_of_file("x/y/f.rs").unwrap()[0], "A <a@x>");
        assert_eq!(map.stewards_of_file("x/f.rs").unwrap()[0], "A <a@x>");
        assert_eq!(map.stewards_of_file("other/f.rs").unwrap()[0], "Root <r@x>");
        assert_eq!(map.stewards_of_file("f.rs").unwrap()[0], "Root <r@x>");
    }

    #[test]
    fn no_root_stew_means_unowned() {
        let mut map = StewMap::default();
        map.dirs
            .insert(PathBuf::from("x"), vec!["A <a@x>".into()]);
        assert!(map.stewards_of_file("f.rs").is_none());
        assert!(map.stewards_of_file("x/f.rs").is_some());
    }

    #[test]
    fn tree_nests_transitions() {
        let mut map = StewMap::default();
        map.dirs
            .insert(PathBuf::from(""), vec!["Root <r@x>".into()]);
        map.dirs
            .insert(PathBuf::from("x"), vec!["A <a@x>".into()]);
        map.dirs
            .insert(PathBuf::from("x/y/z"), vec!["B <b@x>".into()]);
        map.dirs
            .insert(PathBuf::from("docs"), vec!["C <c@x>".into()]);

        let tree = build_tree(&map);
        assert_eq!(tree.stewards, vec!["Root <r@x>"]);
        assert_eq!(tree.children.len(), 2);
        let x = tree
            .children
            .iter()
            .find(|c| c.rel == Path::new("x"))
            .unwrap();
        assert_eq!(x.children.len(), 1);
        assert_eq!(x.children[0].rel, Path::new("x/y/z"));
    }
}
