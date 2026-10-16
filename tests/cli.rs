//! End-to-end tests driving the `stew` binary against a scratch project.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn stew() -> Command {
    Command::new(env!("CARGO_BIN_EXE_stew"))
}

/// Build a little project:
///   root (stew.toml, .stew → Root)
///   ├── x (.stew → A): one.rs (2 lines)
///   │   └── y/z (.stew → B): two.rs (3 lines)
///   └── plain.txt (1 line, Root's)
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("stew.toml"), "").unwrap();
    fs::write(root.join(".stew"), "Root <r@example.com>\n").unwrap();
    fs::write(root.join("plain.txt"), "hello\n").unwrap();
    fs::create_dir_all(root.join("x/y/z")).unwrap();
    fs::write(root.join("x/.stew"), "# owner\nA <a@example.com>\n").unwrap();
    fs::write(root.join("x/one.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    fs::write(root.join("x/y/z/.stew"), "B <b@example.com>\n").unwrap();
    fs::write(root.join("x/y/z/two.rs"), "1\n2\n3\n").unwrap();
    dir
}

fn run(root: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let mut cmd = stew();
    cmd.arg("--root").arg(root).args(args);
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    cmd.stdout(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "stew {args:?} failed");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn tree_shows_transitions() {
    let dir = scratch();
    let out = run(dir.path(), &["tree"], None);
    assert!(out.contains(". — Root <r@example.com>"), "{out}");
    assert!(out.contains("x — A <a@example.com>"), "{out}");
    assert!(out.contains("y/z — B <b@example.com>"), "{out}");
}

#[test]
fn stewards_lists_unique_sorted() {
    let dir = scratch();
    fs::write(
        dir.path().join("x/y/.stew"),
        "A <a@example.com>\nRoot <r@example.com>\n",
    )
    .unwrap();
    let out = run(dir.path(), &["stewards"], None);
    assert_eq!(
        out,
        "A <a@example.com>\nB <b@example.com>\nRoot <r@example.com>\n"
    );
}

#[test]
fn stewignore_excludes_files_and_stats_reports_ignored_lines() {
    let dir = scratch();
    let root = dir.path();
    // .gitignore only applies inside a git repo.
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    fs::write(root.join(".gitignore"), "gen.txt\n").unwrap();
    fs::write(root.join("x/gen.txt"), "g1\ng2\ng3\ng4\n").unwrap();
    fs::write(root.join(".stewignore"), "*.lock\n").unwrap();
    fs::write(root.join("big.lock"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    // A nested .stewignore applies to its own subtree.
    fs::write(root.join("x/y/z/.stewignore"), "skip.rs\n").unwrap();
    fs::write(root.join("x/y/z/skip.rs"), "s1\ns2\n").unwrap();

    let out = run(root, &["stats"], None);
    let line = |s: &str| out.lines().find(|l| l.contains(s)).unwrap().to_string();
    let last = |l: String| l.split_whitespace().last().unwrap().to_string();
    // Owned counts are unchanged by the ignored files (Root gains one line
    // for .gitignore itself).
    let count = |l: String| l.split_whitespace().nth(2).unwrap().to_string();
    assert_eq!(count(line("B <b@example.com>")), "3", "{out}");
    assert_eq!(count(line("A <a@example.com>")), "2", "{out}");
    assert_eq!(count(line("Root <r@example.com>")), "2", "{out}");
    assert_eq!(last(line("total")), "7", "{out}");
    assert!(!out.contains(".gitignore"), "{out}");
    assert_eq!(last(line("ignored by .stewignore")), "7", "{out}");

    // unclaimed also respects .stewignore.
    fs::create_dir_all(root.join("free")).unwrap();
    fs::write(root.join("free/.stew"), "").unwrap();
    fs::write(root.join("free/a.lock"), "1\n2\n").unwrap();
    fs::write(root.join("free/a.txt"), "1\n").unwrap();
    let out = run(root, &["unclaimed", &root.display().to_string()], None);
    assert_eq!(
        out.split_whitespace().collect::<Vec<_>>(),
        vec!["free", "1"]
    );
}

#[test]
fn stats_counts_lines_per_steward() {
    let dir = scratch();
    let out = run(dir.path(), &["stats"], None);
    // two.rs: 3 lines → B; one.rs: 2 lines → A; plain.txt: 1 line → Root.
    let line = |s: &str| out.lines().find(|l| l.contains(s)).unwrap().to_string();
    assert!(line("B <b@example.com>").contains('3'), "{out}");
    assert!(line("A <a@example.com>").contains('2'), "{out}");
    assert!(line("Root <r@example.com>").contains('1'), "{out}");
    assert!(line("total").contains('6'), "{out}");
}

#[test]
fn diff_attributes_changes() {
    let dir = scratch();
    let diff = "\
--- a/x/y/z/two.rs
+++ b/x/y/z/two.rs
@@ -1,1 +1,2 @@
-1
+one
+uno
--- a/plain.txt
+++ b/plain.txt
@@ -1,1 +1,1 @@
-hello
+goodbye
";
    let out = run(dir.path(), &["diff"], Some(diff));
    assert!(out.contains("B <b@example.com>  (+2 -1)"), "{out}");
    assert!(out.contains("x/y/z/two.rs  +2 -1"), "{out}");
    assert!(out.contains("Root <r@example.com>  (+1 -1)"), "{out}");
}

/// Like `run`, but returns (success, stdout) and feeds `stdin` if given.
fn try_run(root: &Path, args: &[&str], stdin: Option<&str>) -> (bool, String) {
    let mut cmd = stew();
    cmd.arg("--root").arg(root).args(args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or("").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.success(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn which_prints_governing_stew_file() {
    let dir = scratch();
    // x/y has no .stew of its own; x/.stew governs it.
    let sub = dir.path().join("x/y").display().to_string();
    let out = run(dir.path(), &["which", &sub], None);
    assert_eq!(out.trim(), dir.path().join("x/.stew").display().to_string());
}

#[test]
fn unclaimed_lists_every_unclaimed_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("stew.toml"), "").unwrap();
    // u is unclaimed (empty .stew) but holds a mix of claimed and
    // unclaimed directories, so u itself is not an unclaimed subtree.
    fs::create_dir_all(root.join("u/c1/inner/deeper")).unwrap();
    fs::create_dir_all(root.join("u/c2")).unwrap();
    fs::create_dir_all(root.join("u/n1")).unwrap();
    fs::create_dir_all(root.join("u/n2/deep")).unwrap();
    fs::create_dir_all(root.join("owned")).unwrap();
    fs::write(root.join("u/.stew"), "").unwrap();
    fs::write(root.join("u/c1/.stew"), "A <a@example.com>\n").unwrap();
    fs::write(root.join("u/c2/.stew"), "B <b@example.com>\n").unwrap();
    // An unclaimed directory inside a claimed one.
    fs::write(root.join("u/c1/inner/.stew"), "# nobody\n").unwrap();
    fs::write(root.join("owned/.stew"), "R <r@example.com>\n").unwrap();
    // Lines under each unclaimed subtree are totalled; .stew and binary
    // files don't count.
    fs::write(root.join("u/n2/a.txt"), "1\n2\n").unwrap();
    fs::write(root.join("u/n2/deep/b.txt"), "3\n4\n5\n").unwrap();
    fs::write(root.join("u/n2/deep/bin"), b"\0\n\n").unwrap();
    fs::write(root.join("u/c1/inner/deeper/c.txt"), "x").unwrap();

    let out = run(root, &["unclaimed", &root.display().to_string()], None);
    let mut lines: Vec<Vec<&str>> = out
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        vec![
            vec!["u/c1/inner", "1"],
            vec!["u/n1", "0"],
            vec!["u/n2", "5"]
        ]
    );

    // Starting inside u gives the same set; starting inside an unclaimed
    // subtree lists just the start.
    let out = run(
        root,
        &["unclaimed", &root.join("u").display().to_string()],
        None,
    );
    let mut lines: Vec<&str> = out
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    lines.sort();
    assert_eq!(lines, vec!["u/c1/inner", "u/n1", "u/n2"]);
    let out = run(
        root,
        &["unclaimed", &root.join("u/n2/deep").display().to_string()],
        None,
    );
    assert_eq!(
        out.split_whitespace().collect::<Vec<_>>(),
        vec!["u/n2/deep", "3"]
    );
}

#[test]
fn unclaimed_with_no_root_stew_lists_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("stew.toml"), "").unwrap();
    fs::create_dir_all(root.join("a/b")).unwrap();
    let out = run(root, &["unclaimed", &root.display().to_string()], None);
    assert_eq!(out.split_whitespace().next(), Some("."));
}

#[test]
fn which_fails_when_nothing_governs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("stew.toml"), "").unwrap();
    let root = dir.path().display().to_string();
    let (ok, out) = try_run(dir.path(), &["which", &root], None);
    assert!(!ok);
    assert_eq!(out, "", "stdout must stay clean for pipes");
}

#[test]
fn add_creates_file_and_announces_parent_effect() {
    let dir = scratch();
    let sub = dir.path().join("x/y").display().to_string();
    let out = run(dir.path(), &["add", "C <c@example.com>", &sub, "-y"], None);
    assert!(out.contains("away from x/.stew"), "{out}");
    assert!(out.contains("- A <a@example.com>"), "{out}");
    assert!(
        out.contains("added C <c@example.com> to x/y/.stew"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("x/y/.stew")).unwrap(),
        "C <c@example.com>\n"
    );
}

#[test]
fn add_appends_and_ignores_duplicates() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    run(dir.path(), &["add", "C <c@example.com>", &x, "-y"], None);
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\nA <a@example.com>\nC <c@example.com>\n"
    );
    let out = run(dir.path(), &["add", "A <a@example.com>", &x, "-y"], None);
    assert!(out.contains("already a steward"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\nA <a@example.com>\nC <c@example.com>\n"
    );
}

#[test]
fn add_quiet_suppresses_result_line() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    let out = run(
        dir.path(),
        &["add", "C <c@example.com>", &x, "-y", "-q"],
        None,
    );
    assert!(!out.contains("added"), "{out}");
}

#[test]
fn add_prompt_can_be_declined() {
    let dir = scratch();
    let sub = dir.path().join("x/y").display().to_string();
    let (ok, out) = try_run(dir.path(), &["add", "C <c@example.com>", &sub], Some("n\n"));
    assert!(!ok);
    assert!(out.contains("Proceed? [y/N]"), "{out}");
    assert!(!dir.path().join("x/y/.stew").exists());

    let (ok, _) = try_run(dir.path(), &["add", "C <c@example.com>", &sub], Some("y\n"));
    assert!(ok);
    assert!(dir.path().join("x/y/.stew").exists());
}

#[test]
fn remove_last_steward_leaves_empty_file_meaning_unowned() {
    let dir = scratch();
    let z = dir.path().join("x/y/z").display().to_string();
    let out = run(dir.path(), &["remove", "B <b@example.com>", &z, "-y"], None);
    assert!(out.contains("leaves x/y/z/.stew empty"), "{out}");
    assert!(out.contains("fall back to x/.stew"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x/y/z/.stew")).unwrap(),
        ""
    );

    // The empty file is honoured: two.rs is now unowned, not A's.
    let out = run(dir.path(), &["stats"], None);
    let line = |s: &str| out.lines().find(|l| l.contains(s)).unwrap().to_string();
    assert!(line("(unclaimed)").contains('3'), "{out}");
    assert!(line("A <a@example.com>").contains('2'), "{out}");
    let out = run(dir.path(), &["tree"], None);
    assert!(out.contains("y/z — (no steward)"), "{out}");
}

#[test]
fn remove_keeps_comments_and_other_stewards() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    run(dir.path(), &["add", "C <c@example.com>", &x, "-y"], None);
    run(dir.path(), &["remove", "A <a@example.com>", &x, "-y"], None);
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\nC <c@example.com>\n"
    );
}

#[test]
fn me_uses_git_config_and_rm_is_an_alias() {
    let dir = scratch();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Me Myself"]);
    git(&["config", "user.email", "me@example.com"]);

    let x = dir.path().join("x").display().to_string();
    let out = run(dir.path(), &["add", "me", &x, "-y"], None);
    assert!(
        out.contains("added Me Myself <me@example.com> to x/.stew"),
        "{out}"
    );
    let out = run(dir.path(), &["rm", "me", &x, "-y"], None);
    assert!(
        out.contains("removed Me Myself <me@example.com> from x/.stew"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\nA <a@example.com>\n"
    );
}

#[test]
fn remove_without_file_is_an_error() {
    let dir = scratch();
    let sub = dir.path().join("x/y").display().to_string();
    let (ok, _) = try_run(
        dir.path(),
        &["remove", "A <a@example.com>", &sub, "-y"],
        None,
    );
    assert!(!ok);
}

#[test]
fn missing_manifest_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let status = stew()
        .arg("--root")
        .arg(dir.path())
        .arg("tree")
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
}
