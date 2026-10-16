//! Minimal unified-diff parser: extracts per-file added/removed line counts.

/// Per-file change counts extracted from a unified diff.
#[derive(Debug, PartialEq, Eq)]
pub struct FileDiff {
    /// Path relative to the project root (the `b/` side, or the `a/` side
    /// for deletions).
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

/// Strip a `a/` or `b/` prefix as produced by `git diff`.
fn strip_prefix(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

/// Parse a unified diff (as produced by `git diff` or `diff -u`) into
/// per-file counts. Unrecognised lines are ignored, so extended headers,
/// binary notices and mode changes pass through harmlessly.
pub fn parse(input: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_path: Option<String> = None;
    let mut in_hunk = false;

    for line in input.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            let p = rest.split('\t').next().unwrap_or(rest);
            old_path = (p != "/dev/null").then(|| strip_prefix(p).to_string());
            in_hunk = false;
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let p = rest.split('\t').next().unwrap_or(rest);
            let path = if p == "/dev/null" {
                // Deletion: fall back to the old path.
                old_path.take()
            } else {
                Some(strip_prefix(p).to_string())
            };
            if let Some(path) = path {
                files.push(FileDiff {
                    path,
                    added: 0,
                    removed: 0,
                });
            }
            in_hunk = false;
        } else if line.starts_with("@@") {
            in_hunk = !files.is_empty();
        } else if in_hunk {
            if let Some(f) = files.last_mut() {
                if line.starts_with('+') {
                    f.added += 1;
                } else if line.starts_with('-') {
                    f.removed += 1;
                } else if !line.starts_with(' ') && !line.starts_with('\\') {
                    // A non-hunk line (e.g. the next "diff --git" header)
                    // ends the hunk.
                    in_hunk = false;
                }
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/x/y/f.rs b/x/y/f.rs
index 111..222 100644
--- a/x/y/f.rs
+++ b/x/y/f.rs
@@ -1,3 +1,4 @@
 fn main() {
-    old();
+    new();
+    extra();
 }
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-bye
-bye
";

    #[test]
    fn parses_modifications_and_deletions() {
        let files = parse(DIFF);
        assert_eq!(
            files,
            vec![
                FileDiff {
                    path: "x/y/f.rs".into(),
                    added: 2,
                    removed: 1
                },
                FileDiff {
                    path: "gone.txt".into(),
                    added: 0,
                    removed: 2
                },
            ]
        );
    }

    #[test]
    fn new_file_uses_b_side() {
        let diff = "--- /dev/null\n+++ b/fresh.rs\n@@ -0,0 +1,2 @@\n+a\n+b\n";
        let files = parse(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "fresh.rs");
        assert_eq!(files[0].added, 2);
    }
}
