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
