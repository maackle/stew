//! The stewardship model: locating the project root, parsing `.stew` files,
//! and building the tree of stewardship transitions.

use anyhow::{bail, Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const ROOT_MANIFEST: &str = "stew.toml";
pub const STEW_FILE: &str = ".stew";
pub const STEW_IGNORE: &str = ".stewignore";

/// A steward as it appears in a `.stew` file: a member key from the
/// `[members]` table of `stew.toml`, e.g. `jane`. Kept as the raw (trimmed)
/// line; `check` verifies that every key is a known member.
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

/// One entry of `[members]`.
#[derive(Debug, Clone)]
pub struct Member {
    /// The git identities this member commits as, e.g.
    /// `Jane Doe <jane@example.com>`.
    pub names: Vec<String>,
    /// Patterns, in `.gitignore` syntax relative to the project root, for
    /// files this member stewards regardless of `.stew` files, as written.
    pub overrides: Vec<String>,
    /// `overrides`, compiled.
    pub override_set: Gitignore,
}

impl Member {
    pub fn new(names: Vec<String>, overrides: Vec<String>) -> Result<Self> {
        let mut b = GitignoreBuilder::new("");
        for pat in &overrides {
            b.add_line(None, pat)
                .with_context(|| format!("invalid override pattern {pat:?}"))?;
        }
        let override_set = b.build().context("invalid override patterns")?;
        Ok(Member {
            names,
            overrides,
            override_set,
        })
    }

    /// Whether `rel` (a root-relative file path) is claimed by an override.
    pub fn overrides_file(&self, rel: &str) -> bool {
        !self.overrides.is_empty()
            && self
                .override_set
                .matched_path_or_any_parents(Path::new(rel), false)
                .is_ignore()
    }
}

/// The contents of `stew.toml`.
#[derive(Debug, Default, Clone)]
pub struct Manifest {
    /// Member keys (as written in `.stew` files) to their definitions.
    pub members: BTreeMap<String, Member>,
}

impl Manifest {
    /// The member key whose identities include `identity`, if any.
    pub fn member_for_identity(&self, identity: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|(_, m)| m.names.iter().any(|i| i == identity))
            .map(|(k, _)| k.as_str())
    }

    /// The members whose overrides claim `rel`, in key order. Empty when no
    /// override applies and the `.stew` files decide.
    pub fn overriders(&self, rel: &str) -> Vec<Steward> {
        self.members
            .iter()
            .filter(|(_, m)| m.overrides_file(rel))
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// A `[members]` value: a string, a list of strings, or a table with
/// `name`/`names` and optionally `override`/`overrides`.
fn parse_member(value: &toml::Value) -> Result<Member> {
    match value {
        toml::Value::Table(t) => {
            for k in t.keys() {
                if !matches!(k.as_str(), "name" | "names" | "override" | "overrides") {
                    bail!("unknown field `{k}` (expected name/names, override/overrides)");
                }
            }
            let names = one_or_many(t, "name", "names")?;
            if names.is_empty() {
                bail!("needs `name` or `names`");
            }
            let overrides = one_or_many(t, "override", "overrides")?;
            Member::new(names, overrides)
        }
        _ => Member::new(strings(value, "value")?, Vec::new()),
    }
}

/// Read `single` (a string) or `plural` (a list of strings) from `t`, but
/// not both. Absent means empty.
fn one_or_many(t: &toml::Table, single: &str, plural: &str) -> Result<Vec<String>> {
    match (t.get(single), t.get(plural)) {
        (Some(_), Some(_)) => bail!("give either `{single}` or `{plural}`, not both"),
        (Some(v), None) => strings(v, single),
        (None, Some(v)) => strings(v, plural),
        (None, None) => Ok(Vec::new()),
    }
}

/// A string or list of strings, all non-empty.
fn strings(value: &toml::Value, what: &str) -> Result<Vec<String>> {
    let out: Vec<String> = match value {
        toml::Value::String(s) => vec![s.clone()],
        toml::Value::Array(a) => a
            .iter()
            .map(|v| match v {
                toml::Value::String(s) => Ok(s.clone()),
                _ => bail!("`{what}`: every entry must be a string"),
            })
            .collect::<Result<_>>()?,
        _ => bail!("`{what}` must be a string or a list of strings"),
    };
    if out.iter().any(|i| i.trim().is_empty()) {
        bail!("`{what}`: entries must not be empty");
    }
    Ok(out)
}

/// A member key: alphanumeric characters and underscores only.
pub fn is_valid_member_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Load and validate the root manifest. The `version` field must equal
/// this binary's version; `[members]` (optional) maps member keys to one or
/// more git identities.
pub fn load_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join(ROOT_MANIFEST);
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    parse_manifest(&text).with_context(|| format!("invalid {}", path.display()))
}

pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let table: toml::Table = text.parse().context("not valid TOML")?;

    let want = env!("CARGO_PKG_VERSION");
    match table.get("version") {
        None => bail!("missing `version` field (this is stew {want}, so add `version = \"{want}\"`)"),
        Some(toml::Value::String(v)) if v == want => {}
        Some(toml::Value::String(v)) => {
            bail!("version {v} does not match this stew binary, which is version {want}")
        }
        Some(_) => bail!("`version` must be a string"),
    }

    let mut members = BTreeMap::new();
    match table.get("members") {
        None => {}
        Some(toml::Value::Table(t)) => {
            for (key, value) in t {
                if !is_valid_member_key(key) {
                    bail!("member key {key:?} is not valid: use only letters, digits and `_`");
                }
                let member =
                    parse_member(value).with_context(|| format!("member {key}"))?;
                members.insert(key.clone(), member);
            }
        }
        Some(_) => bail!("`members` must be a table"),
    }
    Ok(Manifest { members })
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
    pub manifest: Manifest,
    pub dirs: BTreeMap<PathBuf, Vec<Steward>>,
}

impl StewMap {
    /// Scan the project (respecting `.gitignore` etc.) and collect all
    /// `.stew` files.
    pub fn load(root: &Path) -> Result<Self> {
        let manifest = load_manifest(root)?;
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
            manifest,
            dirs,
        })
    }

    /// Every `(directory, key)` where a `.stew` file names a key that is
    /// not in `[members]`, in path order. Empty when the project is
    /// consistent.
    pub fn unknown_members(&self) -> Vec<(&Path, &Steward)> {
        self.dirs
            .iter()
            .flat_map(|(dir, stewards)| stewards.iter().map(move |s| (dir.as_path(), s)))
            .filter(|(_, s)| !self.manifest.members.contains_key(*s))
            .collect()
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

    /// The stewards owning a file path (relative to the root): the members
    /// whose overrides match it, or else those named by the nearest `.stew`
    /// at or above its directory. `None` if neither applies.
    pub fn stewards_of_file(&self, rel: &str) -> Option<Vec<Steward>> {
        let overriders = self.manifest.overriders(rel);
        if !overriders.is_empty() {
            return Some(overriders);
        }
        self.stewards_by_stew(rel).map(<[Steward]>::to_vec)
    }

    /// Whether some member's override claims `rel`.
    pub fn is_overridden(&self, rel: &str) -> bool {
        self.manifest.members.values().any(|m| m.overrides_file(rel))
    }

    /// The stewards named for a file by `.stew` files alone.
    fn stewards_by_stew(&self, rel: &str) -> Option<&[Steward]> {
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
    fn manifest_parses_members() {
        let m = parse_manifest(&format!(
            "version = \"{}\"\n[members]\na = \"A <a@x>\"\nb_2 = [\"B <b@x>\", \"Bee <bee@x>\"]\nc = {{ name = \"C <c@x>\", overrides = [\"*.lock\", \"docs/\"] }}\n",
            env!("CARGO_PKG_VERSION")
        ))
        .unwrap();
        assert_eq!(m.members["a"].names, vec!["A <a@x>"]);
        assert_eq!(m.members["b_2"].names, vec!["B <b@x>", "Bee <bee@x>"]);
        assert_eq!(m.members["c"].names, vec!["C <c@x>"]);
        assert_eq!(m.overriders("Cargo.lock"), vec!["c"]);
        assert_eq!(m.overriders("docs/a/b.md"), vec!["c"]);
        assert!(m.overriders("src/main.rs").is_empty());
        assert_eq!(m.member_for_identity("Bee <bee@x>"), Some("b_2"));
        assert_eq!(m.member_for_identity("Nobody <n@x>"), None);
    }

    #[test]
    fn manifest_requires_matching_version() {
        assert!(parse_manifest("").is_err());
        assert!(parse_manifest("version = \"0.0.0-nope\"").is_err());
        assert!(parse_manifest(&format!("version = \"{}\"", env!("CARGO_PKG_VERSION"))).is_ok());
    }

    #[test]
    fn overrides_beat_stew_files() {
        let mut map = StewMap::default();
        map.manifest
            .members
            .insert("a".into(), Member::new(vec!["A <a@x>".into()], vec![]).unwrap());
        map.manifest.members.insert(
            "lock".into(),
            Member::new(vec!["L <l@x>".into()], vec!["*.lock".into()]).unwrap(),
        );
        map.dirs.insert(PathBuf::from(""), vec!["a".into()]);
        assert_eq!(map.stewards_of_file("x/f.rs").unwrap(), vec!["a"]);
        assert_eq!(map.stewards_of_file("x/Cargo.lock").unwrap(), vec!["lock"]);
        assert!(map.is_overridden("Cargo.lock"));
        assert!(!map.is_overridden("f.rs"));
    }

    #[test]
    fn member_key_charset() {
        assert!(is_valid_member_key("jane_doe2"));
        assert!(!is_valid_member_key(""));
        assert!(!is_valid_member_key("jane-doe"));
        assert!(!is_valid_member_key("Jane Doe <j@x>"));
    }

    #[test]
    fn unknown_members_lists_keys_not_in_manifest() {
        let mut map = StewMap::default();
        map.manifest.members.insert("a".into(), Member::new(vec!["A <a@x>".into()], vec![]).unwrap());
        map.dirs.insert(PathBuf::from(""), vec!["a".into(), "ghost".into()]);
        map.dirs.insert(PathBuf::from("x"), vec!["a".into()]);
        let bad = map.unknown_members();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, Path::new(""));
        assert_eq!(bad[0].1, "ghost");
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
