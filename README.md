# stew

A stewardship linter. `.stew` files scattered through a project define who
"stewards" each subtree, and `stew` does calculations on the resulting tree.

## The model

- The project root is the directory containing `stew.toml` (currently a
  placeholder with no settings; `stew` searches upward from the current
  directory to find it, or takes `--root`).
- Any directory may contain a `.stew` file: one git identifier per line, as
  it would appear in a commit (`Jane Doe <jane@example.com>`). Blank lines
  and `#` comments are ignored.
- The stewards named in a `.stew` file own that directory's subtree, down to
  (but not including) any deeper directory with its own `.stew` file.
- Files above the shallowest `.stew` files are unowned.

## Commands

### `stew tree`

Shows the tree of stewardship *transitions*, in the style of `cargo tree`.
Directories where stewardship doesn't change hands are collapsed into the
path label, so if `./x/.stew` names A and `./x/y/z/.stew` names B:

```
. — Root Steward <root@example.com>
└── x — A <a@example.com>
    └── y/z — B <b@example.com>
```

### `stew stats`

Lines of code owned per steward and each steward's share of the project
total. Walks the tree respecting `.gitignore`, skips binary files and the
`.stew`/`stew.toml` metadata files themselves. When a directory names
multiple stewards, its lines count fully toward each of them.

### `stew show [DIR]`

Shows the stewards of a directory (the current directory by default) and the
`.stew` file that names them:

```
$ stew show x/y
directory:  x/y
stewards:   A <a@example.com>
defined by: x/.stew
```

### `stew diff`

Reads a unified diff (`git diff`, `diff -u`) from stdin and groups the
changed files by the stewards that own them, with added/removed line counts:

```
git diff main | stew diff
```
