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
