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
- An *empty* `.stew` file (no stewards, only blanks/comments) is not the same
  as a missing one: it says nobody stewards that directory or anything below
  it, instead of inheriting from the nearest `.stew` above.
- Every command walks the tree respecting `.gitignore`. A `.stewignore` file,
  in gitignore syntax, excludes further paths from `stew`'s view (lockfiles,
  say). Like `.gitignore`, it may appear in any directory and applies to that
  directory's subtree.

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
total. Walks the tree respecting `.gitignore` and `.stewignore`, skips
binary files and the `.stew`/`.stewignore`/`stew.toml` metadata files
themselves. When a directory names multiple stewards, its lines count fully
toward each of them. After the total, it reports how many lines `.stewignore`
left out.

### `stew stewards`

Lists every distinct steward named in any `.stew` file, one per line, in
alphabetical order.

### `stew unclaimed [DIR]`

Lists every unclaimed subtree at or below `DIR` (the current directory by
default), as paths relative to the project root, one per line, each with
the total lines of text under it (counted as in `stew stats`). A directory
is an unclaimed subtree when nothing in it is claimed: no `.stew` governs it
(or the governing one is empty), and no `.stew` at or below it names anyone.
Only the top of each unclaimed subtree is listed, not the directories inside
it. A directory that is itself unowned but contains claimed directories is
not listed; its unclaimed children are listed instead.

### `stew which [DIR]`

Prints the path of the `.stew` file governing a directory (the current
directory by default), and nothing else, so it can be piped:

```
$ cat $(stew which x/y)
A <a@example.com>
```

If no `.stew` file applies, it says so on stderr and exits non-zero.

### `stew add STEWARD DIR` / `stew remove STEWARD DIR`

Add a steward to (or remove one from) `DIR/.stew`. `stew rm` is an alias for
`stew remove`. `stew add me DIR` (likewise `stew remove me DIR`) uses the
identity from git config, `user.name <user.email>`. `add` creates the file if
it doesn't exist; adding an existing steward or removing an absent one is a
no-op. Before making the change, both commands explain what it means for the
tree and ask for confirmation. In particular, creating a new `.stew` takes
the subtree away from the parent `.stew` that currently governs it, and
removing the last steward leaves an empty file, which marks the subtree as
unowned rather than handing it back to the parent:

```
$ stew add "C <c@example.com>" x/y
Create x/y/.stew naming C <c@example.com>.
This takes stewardship of x/y (and everything below it) away from x/.stew, which names:
- A <a@example.com>

Proceed? [y/N] y
added C <c@example.com> to x/y/.stew
```

Flags: `-y`/`--yes` skips the prompt; `-q`/`--quiet` suppresses the
confirmation line printed after the change.

Note, a handy command to list all committers in a git repo is:

```
git shortlog -ens
```

### `stew diff`

Reads a unified diff (`git diff`, `diff -u`) from stdin and groups the
changed files by the stewards that own them, with added/removed line counts:

```
git diff main | stew diff
```
