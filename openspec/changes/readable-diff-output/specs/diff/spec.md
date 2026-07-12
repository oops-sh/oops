# diff — delta for readable-diff-output

## MODIFIED Requirements

### Requirement: Change classification
`oops diff` MUST walk the pending sandbox's upper layer and classify every
changed path as created, modified, or deleted, relative to the lower layer:
a path present in upper but not lower is created; present in both is
modified; an OverlayFS whiteout (character device 0:0) or opaque directory
is deleted. With the `--porcelain` flag, output MUST be one path per line,
prefixed `A ` / `M ` / `D `, sorted by path, never colored, and empty when
there are no changes; this format is the stable machine interface and MUST
NOT change between versions.

#### Scenario: Mixed change set (porcelain)
- **WHEN** a pending sandbox contains a new file `n`, a modified file `m`, and a deleted file `d`, and `oops diff --porcelain` runs
- **THEN** stdout is exactly the lines `D d`, `M m`, `A n` (sorted by path), with no color codes

#### Scenario: Deleted directory tree
- **WHEN** the wrapped command removed directory `sub/` containing files
- **THEN** `oops diff --porcelain` reports `D sub/` (the whiteout), without needing to enumerate the tree that only exists in the lower layer

## ADDED Requirements

### Requirement: Human-readable default output
Without `--porcelain`, `oops diff` MUST group changes into three sections in
this order — Created, Modified, Deleted — each with a heading that includes
the section's count, listing its paths sorted and indented. Empty sections
are omitted. Output MUST end with a one-line summary of the counts
(e.g. `3 created, 1 modified, 2 deleted`), omitting zero counts. When there
are no changes at all, stdout MUST be exactly `no changes`.

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
