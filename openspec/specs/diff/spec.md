# diff Specification

## Purpose
TBD - created by archiving change add-core-sandbox-loop. Update Purpose after archive.
## Requirements
### Requirement: Change classification
`oops diff` MUST walk the pending sandbox's upper layer and classify every
changed path as created, modified, or deleted, relative to the lower layer:
a path present in upper but not lower is created; present in both is
modified; an OverlayFS whiteout (character device 0:0) or opaque directory
is deleted. With the `--porcelain` flag, output MUST be one path per line,
prefixed `A ` / `M ` / `D `, never colored, and empty when there are no
changes; this format is the stable machine interface and MUST NOT change
between versions. The frozen contract includes:

- **Sort order**: entries are sorted by the raw byte order of the path
  (prefix and trailing slash excluded) — not locale- or Unicode-aware
  collation.
- **Directory deletions are not expanded**: a deleted directory is exactly
  one `D <path>/` entry (trailing `/` marks a directory), meaning the whole
  subtree is deleted recursively. Consumers MUST NOT expect per-descendant
  lines; oops never enumerates the lower layer to expand them.

#### Scenario: Mixed change set
- **WHEN** a pending sandbox contains a new file `n`, a modified file `m`, and a deleted file `d`, and `oops diff --porcelain` runs
- **THEN** stdout is exactly the lines `D d`, `M m`, `A n` (byte order of the paths), with no color codes

#### Scenario: Byte-order sorting
- **WHEN** the pending changes include paths `a-b` and `a/c`
- **THEN** `a-b` sorts before `a/c` (byte 0x2d < 0x2f), regardless of locale or path-component structure

#### Scenario: Deleted directory tree
- **WHEN** the wrapped command removed directory `sub/` containing files
- **THEN** `oops diff --porcelain` reports exactly one line `D sub/` and no lines for its former contents

### Requirement: Diff is read-only
`oops diff` MUST NOT modify the upper layer, the lower layer, or the session
record. Running it any number of times MUST leave the pending sandbox
byte-identical.

#### Scenario: Diff twice
- **WHEN** `oops diff` is run twice in a row
- **THEN** both runs produce identical output and the sandbox state is unchanged

### Requirement: Human-readable default output
Without `--porcelain`, `oops diff` MUST group changes into three sections in
this order — Created, Modified, Deleted — each with a heading that includes
the section's count, listing its paths in the same byte order as porcelain,
indented. Empty sections are omitted. Output MUST end with a one-line
summary of the counts (e.g. `3 created, 1 modified, 2 deleted`), omitting
zero counts. When there are no changes at all, stdout MUST be exactly
`no changes`.

#### Scenario: Grouped sections with summary
- **WHEN** a pending sandbox contains new files `n1` and `n2`, a modified file `m`, and a deleted directory `sub/`, and `oops diff` runs
- **THEN** stdout shows a Created section (count 2) listing `n1` and `n2`, a Modified section (count 1) listing `m`, a Deleted section (count 1) listing `sub/`, followed by the summary `2 created, 1 modified, 1 deleted`

#### Scenario: Empty diff
- **WHEN** the wrapped command changed nothing and `oops diff` runs
- **THEN** stdout is `no changes` and the exit status is 0

### Requirement: Color behavior
Human-mode output SHALL use ANSI colors (created green, modified yellow,
deleted red) only when stdout is a terminal. Color MUST be disabled when
stdout is not a TTY, when the `NO_COLOR` environment variable is set
(any value), or when `--porcelain` is given.

#### Scenario: Piped output has no escape codes
- **WHEN** `oops diff` runs with stdout redirected to a file or pipe
- **THEN** the output contains no ANSI escape sequences

#### Scenario: NO_COLOR wins over a TTY
- **WHEN** `oops diff` runs on a TTY with `NO_COLOR=1` in the environment
- **THEN** the output contains no ANSI escape sequences

### Requirement: Diff exit semantics
`oops diff` MUST exit 0 whenever the diff was produced successfully,
regardless of whether any changes exist. The exit code carries no
empty-vs-non-empty signal (consumers inspect the output); non-zero is
reserved for oops-level failures (no pending sandbox, stale session,
unreadable upper layer) per the cli capability.

#### Scenario: Non-empty diff exits 0
- **WHEN** `oops diff` runs against a sandbox with pending changes
- **THEN** the exit status is 0

### Requirement: Newline-unsafe paths are a known limitation
The line-oriented porcelain format cannot represent paths containing
newline bytes; this MUST be documented as a known limitation rather than
escaped or mangled ad hoc. The flag name `-z` is reserved for a future
NUL-terminated porcelain variant (mirroring `git status -z`) and MUST NOT
be used for anything else.

#### Scenario: Reserved flag is not squatted
- **WHEN** any future change adds a short flag to `oops diff`
- **THEN** `-z` is either the NUL-terminated output variant or still unused

