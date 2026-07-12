# diff — delta for add-apfs-backend

Porcelain **output bytes are unchanged** by this delta; the frozen format
survives intact. The revisions replace implementation claims with output
contracts and add an explicit cost model.

## MODIFIED Requirements

### Requirement: Change classification
`oops diff` MUST classify every changed path as created, modified, or
deleted, using the backend's change source: for OverlayFS, the upper layer
(whiteouts and opaque directories mark deletions); for APFS, a comparison
of the snapshot against the live tree. With the `--porcelain` flag, output
MUST be one path per line, prefixed `A ` / `M ` / `D `, never colored, and
empty when there are no changes; this format is the stable machine
interface and MUST NOT change between versions. The frozen contract
includes:

- **Sort order**: entries are sorted by the raw byte order of the path
  (prefix and trailing slash excluded) — not locale- or Unicode-aware
  collation.
- **Directory deletions are not expanded**: a deleted directory is exactly
  one `D <path>/` entry (trailing `/` marks a directory), meaning the whole
  subtree is deleted recursively. Consumers MUST NOT expect per-descendant
  lines; descendants of a deleted directory are never listed. (Whether a
  backend internally enumerates a snapshot to detect the deletion is an
  implementation matter, not part of this contract.)

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
`oops diff` MUST NOT modify the target tree, any snapshot or layer, or the
session record. Running it any number of times MUST leave the pending
sandbox byte-identical.

#### Scenario: Diff twice
- **WHEN** `oops diff` is run twice in a row
- **THEN** both runs produce identical output and the sandbox state is unchanged

## ADDED Requirements

### Requirement: Diff cost model
`oops diff` MAY cost O(tree) metadata operations (the APFS backend performs
a pruned two-way stat walk of snapshot and live tree; measured ~140ms at
10k files, ~790ms at 50k). It MUST NOT read file contents by default.
Modification detection on snapshot-restore backends uses size plus
nanosecond mtime; the known gap — a command that rewrites a file and forges
back identical size and mtime escapes detection — MUST be documented as a
limitation (a future `--verify` content comparison is backlog).

#### Scenario: Diff does not read contents
- **WHEN** `oops diff` runs on the apfs backend over a large tree
- **THEN** the diff completes using metadata only, without opening file contents

#### Scenario: Heuristic gap is documented
- **WHEN** a user consults the documentation about diff accuracy on macOS
- **THEN** the size+mtime heuristic and its forged-metadata limitation are stated
