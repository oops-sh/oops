# sandbox — delta for add-apfs-backend

## MODIFIED Requirements

### Requirement: SnapshotBackend trait
The sandbox layer MUST be defined as a trait (`SnapshotBackend`) so backends
are interchangeable. The trait MUST cover the full sandbox lifecycle:
prepare (create the snapshot or copy-on-write layer for a target
directory), execute (run a command under the backend's protection model),
discard (undo), and merge (commit). Two models are recognized:
**interception** (OverlayFS: writes never reach the real tree) and
**snapshot-restore** (APFS: the real tree is mutated and can be atomically
restored). Backend selection: Linux → `overlayfs`; macOS with an APFS
target → `apfs`; otherwise selection MUST fail (which, per safety, aborts
the run before the command executes).

#### Scenario: Backend selection on Linux
- **WHEN** oops starts on Linux with OverlayFS available
- **THEN** the overlayfs backend is selected

#### Scenario: Backend selection on macOS
- **WHEN** `oops run` is invoked on macOS with the target on an APFS volume
- **THEN** the apfs backend is selected and no root privileges are required

#### Scenario: No backend available
- **WHEN** oops starts on a platform where no backend reports itself usable
- **THEN** backend selection returns an error (which, per safety, aborts the run before the command executes)

### Requirement: Writes are redirected to the upper layer
For the **OverlayFS (interception) backend**, `run` MUST execute the
command in a context (mount namespace) where the target directory is an
OverlayFS mount whose lower layer is the real directory. All creates,
modifications, and deletions performed by the command MUST land in the
upper layer; the lower layer MUST remain byte-identical to its pre-run
state. (The snapshot-restore backend has no equivalent guarantee — see
"APFS snapshot-restore backend".)

#### Scenario: Command creates a file
- **WHEN** `oops run "echo hi > new.txt"` completes in directory D on the overlayfs backend
- **THEN** `new.txt` exists in the upper layer and does not exist in the real D

#### Scenario: Command deletes a tree
- **WHEN** `oops run "rm -rf sub/"` completes in directory D containing `sub/` on the overlayfs backend
- **THEN** the upper layer contains a whiteout for `sub` and the real `D/sub` still exists with identical contents

### Requirement: Commit partial-failure semantics
For the **OverlayFS backend**, `commit` is a fail-stop, idempotent replay.
On the first error it MUST stop, report the failing path and
applied/remaining counts, exit non-zero, and preserve the session record
and upper layer so that re-running `commit` completes the merge. Before
modifying the real tree, commit MUST verify the upper layer contains no
unrecognized overlay xattrs (anything beyond whiteouts and
`trusted.overlay.opaque`) and abort if it does. (The APFS backend's commit
performs no replay — see "Snapshot-restore commit".)

#### Scenario: Commit fails midway
- **WHEN** `oops commit` fails partway through the replay (e.g. a permission error on one path)
- **THEN** oops exits non-zero naming the failing path, the session and upper layer remain intact, and a subsequent `oops commit` (after fixing the cause) completes the merge with the same end state as a single successful commit

#### Scenario: Unrecognized overlay metadata
- **WHEN** the upper layer contains an overlay xattr commit does not recognize
- **THEN** commit aborts before modifying any real path and exits non-zero explaining what it found

### Requirement: Undo performance
Discarding a sandbox MUST be O(size of changes) or better, never O(size of
the tree): `oops undo` MUST complete in under 100ms for a repo-sized tree
(~10k files) with a small change set. This MUST be verified by a benchmark
per backend: the OverlayFS benchmark runs inside the test container; the
APFS benchmark runs on a macOS host.

#### Scenario: Benchmark exists and passes
- **WHEN** the benchmark runs in the container against a generated ~10k-file tree after `oops run "rm -rf <subtree>"`
- **THEN** the measured `undo` wall time is under 100ms

#### Scenario: APFS benchmark exists and passes
- **WHEN** the APFS undo benchmark runs on a macOS host against a generated ~10k-file tree after `oops run "rm -rf <subtree>"`
- **THEN** the measured `undo` wall time is under 100ms

## ADDED Requirements

### Requirement: APFS snapshot-restore backend
On the apfs backend, `run` MUST take a `clonefile(2)` snapshot of the
target tree into a registered state root **before** the command starts,
record the target's canonical path and its **parent directory's** identity
(`st_dev`, `st_ino`), and then execute the command against the real tree.
`undo` MUST restore only after re-verifying the recorded parent identity,
following the three branches defined in the safety spec's undo-containment
requirement (existing non-symlink target → `renamex_np(RENAME_SWAP)`;
missing target → rename the snapshot into the verified parent; symlink
target → refuse).
The snapshot and the target MUST be on the same volume; if the clone
cannot be created (cross-volume, ENOSPC, non-APFS filesystem), `run` MUST
refuse to execute the command (fail closed). Cloning is not atomic against
concurrent writers; this MUST be documented as fine print.

#### Scenario: Flagship demo on macOS
- **WHEN** a directory tree exists and `oops run "rm -rf testdir"` then `oops undo` are executed on the apfs backend
- **THEN** after run the real `testdir` is gone; after undo the tree is byte-identical to its pre-run state

#### Scenario: Clone failure refuses to run
- **WHEN** the snapshot cannot be created (e.g. the target is not on an APFS volume)
- **THEN** the command is never executed and oops exits non-zero explaining the failure

### Requirement: Snapshot-restore commit
On the apfs backend, `commit` MUST NOT modify the target tree (it already
holds the command's changes); it consumes the session by moving the
snapshot into trash for asynchronous deletion. Commit is O(1) and MUST
complete in under 100ms regardless of tree size.

#### Scenario: Commit keeps the mutated tree
- **WHEN** `oops run "touch new"` then `oops commit` run on the apfs backend
- **THEN** `new` exists in the real tree, the session is consumed, and no file content was copied during commit

### Requirement: Sandbox setup cost
Sandbox setup (`run` startup, before the command executes) MAY cost O(tree)
— e.g. the APFS whole-tree clone, measured at ~100ms for 10k files and
~600ms for 50k files. Each backend's setup cost model MUST be documented,
and setup expected to exceed one second MUST emit progress feedback to
stderr rather than stalling silently. Setup cost MUST be reported by the
backend benchmark alongside undo time. Optimizations (privileged
`fs_snapshot` fast path, FSEvents-assisted diff) are backlog, not
requirements.

#### Scenario: Large tree gives feedback
- **WHEN** `oops run` prepares a sandbox whose setup takes longer than one second
- **THEN** oops prints a progress line to stderr before the command starts
