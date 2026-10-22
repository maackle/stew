//! End-to-end tests driving the `stew` binary against a scratch project.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn stew() -> Command {
    Command::new(env!("CARGO_BIN_EXE_stew"))
}

const MANIFEST: &str = "\
version = \"0.1.0\"

[members]
root = \"Root <r@example.com>\"
a = \"A <a@example.com>\"
b = [\"B <b@example.com>\", \"Bee <bee@example.com>\"]
c = \"C <c@example.com>\"
myself = \"Me Myself <me@example.com>\"
";

/// Build a little project:
///   root (stew.toml, .stew → root)
///   ├── x (.stew → a): one.rs (2 lines)
///   │   └── y/z (.stew → b): two.rs (3 lines)
///   └── plain.txt (1 line, root's)
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("stew.toml"), MANIFEST).unwrap();
    fs::write(root.join(".stew"), "root\n").unwrap();
    fs::write(root.join("plain.txt"), "hello\n").unwrap();
    fs::create_dir_all(root.join("x/y/z")).unwrap();
    fs::write(root.join("x/.stew"), "# owner\na\n").unwrap();
    fs::write(root.join("x/one.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    fs::write(root.join("x/y/z/.stew"), "b\n").unwrap();
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
    assert!(out.contains(". — root"), "{out}");
    assert!(out.contains("x — a"), "{out}");
    assert!(out.contains("y/z — b"), "{out}");
}

#[test]
fn stewards_lists_unique_sorted() {
    let dir = scratch();
    fs::write(
        dir.path().join("x/y/.stew"),
        "a\nroot\n",
    )
    .unwrap();
    let out = run(dir.path(), &["stewards"], None);
    assert_eq!(
        out,
        "a\nb\nroot\n"
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
    assert_eq!(count(line("b")), "3", "{out}");
    assert_eq!(count(line("a")), "2", "{out}");
    assert_eq!(count(line("root")), "2", "{out}");
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
    assert!(line("b").contains('3'), "{out}");
    assert!(line("a").contains('2'), "{out}");
    assert!(line("root").contains('1'), "{out}");
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
    assert!(out.contains("b  (+2 -1)"), "{out}");
    assert!(out.contains("x/y/z/two.rs  +2 -1"), "{out}");
    assert!(out.contains("root  (+1 -1)"), "{out}");
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
    fs::write(root.join("stew.toml"), MANIFEST).unwrap();
    // u is unclaimed (empty .stew) but holds a mix of claimed and
    // unclaimed directories, so u itself is not an unclaimed subtree.
    fs::create_dir_all(root.join("u/c1/inner/deeper")).unwrap();
    fs::create_dir_all(root.join("u/c2")).unwrap();
    fs::create_dir_all(root.join("u/n1")).unwrap();
    fs::create_dir_all(root.join("u/n2/deep")).unwrap();
    fs::create_dir_all(root.join("owned")).unwrap();
    fs::write(root.join("u/.stew"), "").unwrap();
    fs::write(root.join("u/c1/.stew"), "a\n").unwrap();
    fs::write(root.join("u/c2/.stew"), "b\n").unwrap();
    // An unclaimed directory inside a claimed one.
    fs::write(root.join("u/c1/inner/.stew"), "# nobody\n").unwrap();
    fs::write(root.join("owned/.stew"), "root\n").unwrap();
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
    fs::write(root.join("stew.toml"), MANIFEST).unwrap();
    fs::create_dir_all(root.join("a/b")).unwrap();
    let out = run(root, &["unclaimed", &root.display().to_string()], None);
    assert_eq!(out.split_whitespace().next(), Some("."));
}

#[test]
fn which_fails_when_nothing_governs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("stew.toml"), MANIFEST).unwrap();
    let root = dir.path().display().to_string();
    let (ok, out) = try_run(dir.path(), &["which", &root], None);
    assert!(!ok);
    assert_eq!(out, "", "stdout must stay clean for pipes");
}

#[test]
fn add_creates_file_and_announces_parent_effect() {
    let dir = scratch();
    let sub = dir.path().join("x/y").display().to_string();
    let out = run(dir.path(), &["add", "c", &sub, "-y"], None);
    assert!(out.contains("away from x/.stew"), "{out}");
    assert!(out.contains("- a"), "{out}");
    assert!(
        out.contains("added c to x/y/.stew"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("x/y/.stew")).unwrap(),
        "c\n"
    );
}

#[test]
fn add_appends_and_ignores_duplicates() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    run(dir.path(), &["add", "c", &x, "-y"], None);
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\na\nc\n"
    );
    let out = run(dir.path(), &["add", "a", &x, "-y"], None);
    assert!(out.contains("already a steward"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\na\nc\n"
    );
}

#[test]
fn add_quiet_suppresses_result_line() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    let out = run(
        dir.path(),
        &["add", "c", &x, "-y", "-q"],
        None,
    );
    assert!(!out.contains("added"), "{out}");
}

#[test]
fn add_prompt_can_be_declined() {
    let dir = scratch();
    let sub = dir.path().join("x/y").display().to_string();
    let (ok, out) = try_run(dir.path(), &["add", "c", &sub], Some("n\n"));
    assert!(!ok);
    assert!(out.contains("Proceed? [y/N]"), "{out}");
    assert!(!dir.path().join("x/y/.stew").exists());

    let (ok, _) = try_run(dir.path(), &["add", "c", &sub], Some("y\n"));
    assert!(ok);
    assert!(dir.path().join("x/y/.stew").exists());
}

#[test]
fn remove_last_steward_leaves_empty_file_meaning_unowned() {
    let dir = scratch();
    let z = dir.path().join("x/y/z").display().to_string();
    let out = run(dir.path(), &["remove", "b", &z, "-y"], None);
    assert!(out.contains("leaves x/y/z/.stew empty"), "{out}");
    assert!(out.contains("fall back to x/.stew"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x/y/z/.stew")).unwrap(),
        ""
    );

    // The empty file is honoured: two.rs is now unowned, not A's.
    let out = run(dir.path(), &["stats"], None);
    let line = |s: &str| {
        out.lines()
            .find(|l| l.split_whitespace().next() == Some(s))
            .unwrap()
            .to_string()
    };
    assert!(line("(unclaimed)").contains('3'), "{out}");
    assert!(line("a").contains('2'), "{out}");
    let out = run(dir.path(), &["tree"], None);
    assert!(out.contains("y/z — (no steward)"), "{out}");
}

#[test]
fn remove_keeps_comments_and_other_stewards() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    run(dir.path(), &["add", "c", &x, "-y"], None);
    run(dir.path(), &["remove", "a", &x, "-y"], None);
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\nc\n"
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
        out.contains("added myself to x/.stew"),
        "{out}"
    );
    let out = run(dir.path(), &["rm", "me", &x, "-y"], None);
    assert!(
        out.contains("removed myself from x/.stew"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\na\n"
    );
}

#[test]
fn remove_without_file_is_an_error() {
    let dir = scratch();
    let sub = dir.path().join("x/y").display().to_string();
    let (ok, _) = try_run(
        dir.path(),
        &["remove", "a", &sub, "-y"],
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

fn write_manifest(root: &Path, text: &str) {
    fs::write(root.join("stew.toml"), text).unwrap();
}

/// Run and capture (success, stdout, stderr).
fn run_full(root: &Path, args: &[&str]) -> (bool, String, String) {
    let out = stew()
        .arg("--root")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn manifest_version_must_match_binary() {
    let dir = scratch();
    let root = dir.path();
    write_manifest(root, "version = \"9.9.9\"\n");
    let (ok, _, err) = run_full(root, &["tree"]);
    assert!(!ok);
    assert!(err.contains("9.9.9"), "{err}");
    assert!(err.contains(env!("CARGO_PKG_VERSION")), "{err}");

    write_manifest(root, "[members]\n");
    let (ok, _, err) = run_full(root, &["tree"]);
    assert!(!ok);
    assert!(err.contains("version"), "{err}");
}

#[test]
fn manifest_rejects_bad_members() {
    let dir = scratch();
    let root = dir.path();
    for bad in [
        "version = \"0.1.0\"\n[members]\n\"bad-key\" = \"X <x@example.com>\"\n",
        "version = \"0.1.0\"\n[members]\nok = 3\n",
        "version = \"0.1.0\"\n[members]\nok = [\"X <x@example.com>\", 3]\n",
        "version = \"0.1.0\"\n[members]\nok = \"\"\n",
        "version = \"0.1.0\"\nmembers = 3\n",
    ] {
        write_manifest(root, bad);
        let (ok, _, err) = run_full(root, &["tree"]);
        assert!(!ok, "accepted {bad:?}");
        assert!(err.contains("stew.toml"), "{err}");
    }
    // A missing [members] table is allowed (nobody is a member).
    write_manifest(root, "version = \"0.1.0\"\n");
    fs::write(root.join(".stew"), "").unwrap();
    fs::write(root.join("x/.stew"), "").unwrap();
    fs::write(root.join("x/y/z/.stew"), "").unwrap();
    let (ok, _, _) = run_full(root, &["tree"]);
    assert!(ok);
}

#[test]
fn check_reports_unknown_keys_and_gates_other_commands() {
    let dir = scratch();
    let root = dir.path();
    let (ok, out, err) = run_full(root, &["check"]);
    assert!(ok, "{err}");
    assert_eq!(out, "");

    fs::write(root.join("x/.stew"), "a\nnobody\n").unwrap();
    fs::write(root.join("x/y/z/.stew"), "Zed <z@example.com>\n").unwrap();
    let (ok, out, err) = run_full(root, &["check"]);
    assert!(!ok);
    assert_eq!(out, "");
    assert!(err.contains("x/.stew") && err.contains("nobody"), "{err}");
    assert!(err.contains("x/y/z/.stew") && err.contains("Zed <z@example.com>"), "{err}");
    assert!(!err.contains("\"a\""), "{err}");

    // Every other command refuses to run until the check passes.
    for args in [
        vec!["tree"],
        vec!["stats"],
        vec!["stewards"],
        vec!["unclaimed"],
        vec!["which"],
        vec!["add", "c", "x", "-y"],
        vec!["remove", "a", "x", "-y"],
    ] {
        let (ok, out, err) = run_full(root, &args);
        assert!(!ok, "{args:?} ran despite failing check");
        assert_eq!(out, "", "{args:?}");
        assert!(err.contains("nobody"), "{args:?}: {err}");
    }
    assert_eq!(
        fs::read_to_string(root.join("x/.stew")).unwrap(),
        "a\nnobody\n"
    );
}

#[test]
fn add_rejects_unknown_member() {
    let dir = scratch();
    let x = dir.path().join("x").display().to_string();
    let (ok, _, err) = run_full(dir.path(), &["add", "nobody", &x, "-y"]);
    assert!(!ok);
    assert!(err.contains("nobody"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x/.stew")).unwrap(),
        "# owner\na\n"
    );
}

#[test]
fn me_matches_any_identity_of_a_member() {
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
    git(&["config", "user.name", "Bee"]);
    git(&["config", "user.email", "bee@example.com"]);
    let x = dir.path().join("x").display().to_string();
    let out = run(dir.path(), &["add", "me", &x, "-y"], None);
    assert!(out.contains("added b to x/.stew"), "{out}");

    git(&["config", "user.name", "Stranger"]);
    let (ok, _, err) = run_full(dir.path(), &["add", "me", &x, "-y"]);
    assert!(!ok);
    assert!(err.contains("Stranger <bee@example.com>"), "{err}");
}
