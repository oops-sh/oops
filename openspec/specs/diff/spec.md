# diff Specification

## Purpose
TBD - created by archiving change add-core-sandbox-loop. Update Purpose after archive.
## Requirements
### Requirement: Change classification
`oops diff` MUST walk the pending sandbox's upper layer and classify every
changed path as created, modified, or deleted, relative to the lower layer:
a path present in upper but not lower is created; present in both is
modified; an OverlayFS whiteout (character device 0:0) or opaque directory
is deleted. Output MUST be one path per line, prefixed `A ` / `M ` / `D `,
sorted by path. Phase 0 output is plain text only.

#### Scenario: Mixed change set
- **WHEN** a pending sandbox contains a new file `n`, a modified file `m`, and a deleted file `d`, and `oops diff` runs
- **THEN** stdout is exactly `A n`, `D d`, `M m` (sorted), one per line

#### Scenario: Deleted directory tree
- **WHEN** the wrapped command removed directory `sub/` containing files
- **THEN** `oops diff` reports `D sub/` (the whiteout), without needing to enumerate the tree that only exists in the lower layer

### Requirement: Diff is read-only
`oops diff` MUST NOT modify the upper layer, the lower layer, or the session
record. Running it any number of times MUST leave the pending sandbox
byte-identical.

#### Scenario: Diff twice
- **WHEN** `oops diff` is run twice in a row
- **THEN** both runs produce identical output and the sandbox state is unchanged

