# sandbox — SnapshotBackend trait and OverlayFS implementation

## ADDED Requirements

### Requirement: SnapshotBackend trait
The sandbox layer MUST be defined as a trait (`SnapshotBackend`) so backends
are interchangeable. The trait MUST cover the full sandbox lifecycle:
prepare (create the copy-on-write layer over a target directory), execute
(run a command with writes redirected into the layer), discard (drop the
layer), and merge (apply the layer to the real filesystem). OverlayFS is the
only Phase 0 implementation; `apfs` is a planned second implementation and
MUST NOT be required for the trait to make sense.

#### Scenario: Backend selection on Linux
- **WHEN** oops starts on Linux with OverlayFS available
- **THEN** the overlayfs backend is selected

#### Scenario: No backend available
- **WHEN** oops starts on a platform where no backend reports itself usable
- **THEN** backend selection returns an error (which, per safety, aborts the run before the command executes)

### Requirement: Writes are redirected to the upper layer
`run` MUST execute the command in a context (mount namespace) where the
target directory is an OverlayFS mount whose lower layer is the real
directory. All creates, modifications, and deletions performed by the
command MUST land in the upper layer; the lower layer MUST remain
byte-identical to its pre-run state.

#### Scenario: Command creates a file
- **WHEN** `oops run "echo hi > new.txt"` completes in directory D
- **THEN** `new.txt` exists in the upper layer and does not exist in the real D

#### Scenario: Command deletes a tree
- **WHEN** `oops run "rm -rf sub/"` completes in directory D containing `sub/`
- **THEN** the upper layer contains a whiteout for `sub` and the real `D/sub` still exists with identical contents

### Requirement: Command execution semantics
`run` MUST execute the given command via the shell, inside the sandbox, with
the working directory mapped to the sandboxed view of the invocation
directory. oops MUST propagate the command's stdout/stderr and report its
exit status. A non-zero command exit MUST NOT discard the sandbox — the user
decides with `undo` or `commit`.

#### Scenario: Failing command keeps its sandbox
- **WHEN** `oops run "sh -c 'touch a; exit 3'"` completes
- **THEN** oops reports exit status 3, and the pending sandbox with `a` in its upper layer is preserved for diff/undo/commit

### Requirement: Merge fidelity
`commit` MUST apply the upper layer to the real filesystem so the result is
what the command would have produced unsandboxed: created files appear,
modified files carry the new content, whiteouts become real deletions. File
modes MUST be preserved for regular files and directories.

#### Scenario: Commit applies creations, modifications, and deletions
- **WHEN** a pending sandbox contains one created file, one modified file, and one whiteout, and `oops commit` runs
- **THEN** the real directory afterwards contains the created file, the new content of the modified file, and no trace of the deleted path

### Requirement: Undo performance
Discarding a sandbox MUST be O(size of changes), not O(size of the tree):
`oops undo` MUST complete in under 100ms for a repo-sized tree (~10k files)
with a small change set, measured inside the test container by a benchmark.

#### Scenario: Benchmark exists and passes
- **WHEN** the benchmark runs in the container against a generated ~10k-file tree after `oops run "rm -rf <subtree>"`
- **THEN** the measured `undo` wall time is under 100ms
