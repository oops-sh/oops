# safety Specification

## Purpose
TBD - created by archiving change add-core-sandbox-loop. Update Purpose after archive.
## Requirements
### Requirement: Fail-closed sandboxing
oops MUST never make things less safe than not using it. If sandbox setup
fails for any reason (unsupported platform, mount failure, missing
privileges, backend error), `oops run` MUST refuse to execute the command
and exit non-zero. It MUST NOT fall back to running the command unsandboxed.

#### Scenario: OverlayFS mount fails
- **WHEN** `oops run "touch x"` is invoked and the OverlayFS mount cannot be created
- **THEN** the command `touch x` is never executed, `x` does not exist, and oops exits non-zero with an error explaining the sandbox failure

#### Scenario: Unsupported platform
- **WHEN** `oops run "touch x"` is invoked on a platform with no working SnapshotBackend (e.g. macOS in Phase 0)
- **THEN** the command is never executed and oops exits non-zero with a message directing the user to a supported environment

### Requirement: Honest guarantee boundary
The undo guarantee is filesystem-only and target-tree-only: it covers writes
under the single directory tree sandboxed by `oops run` (the invocation
working directory). Writes outside that tree, network side effects, spawned
processes/daemons, and any non-filesystem state are NOT covered. oops MUST
state this boundary in user-facing documentation (README, `run` help text)
and MUST NOT claim or imply undo coverage beyond it.

#### Scenario: Command writes outside the target tree
- **WHEN** `oops run "touch /tmp/outside"` completes and `oops undo` is invoked
- **THEN** `/tmp/outside` still exists — and this behavior is documented as the guarantee boundary, not a bug

### Requirement: Undo containment
`oops undo` MUST modify exactly two things and nothing else:

1. **The session's protected target subtree** — restoring it to its
   recorded snapshot state. The restore is anchored on the **parent
   directory**: at `run` time the session records the parent's `st_dev`
   and `st_ino` plus the target's canonical path; at undo time the parent
   MUST be re-verified against that identity, and on mismatch undo refuses
   and modifies nothing. Within a verified parent, the restore has exactly
   three branches, decided by `lstat` of the target path:
   - **Target exists and is not a symlink** (same inode or a different
     one — a replaced target is treated as just another change the command
     made): restore proceeds normally via the atomic swap.
   - **Target does not exist** (the command deleted it): restore proceeds
     by renaming the snapshot into the verified parent at the recorded
     name.
   - **Target is a symlink**: undo refuses and modifies nothing — a
     symlink at the target path could redirect the restore outside the
     protected scope.
2. **Registered oops state roots** — deletions and renames by undo
   (including its asynchronous deletion phase) and by gc are permitted only
   for paths that a containment check proves to be inside one of the
   registered state roots (the primary root and any per-volume roots listed
   in the registry).

Regardless of session-state corruption or invalid input, undo and gc MUST
NOT remove, truncate, rewrite, or swap any other path on the system.
After a snapshot-restore undo, the displaced tree (the state the command
left behind) sits in a state root's trash awaiting asynchronous deletion:
it is not diffable, not recoverable through any oops command, and the
session is consumed — `oops diff` reports no pending sandbox. Backends
that never modify the target during undo (e.g. OverlayFS, which discards
a layer) automatically satisfy clause 1 by doing nothing to the target.

#### Scenario: Corrupted session state points outside the state directory
- **WHEN** the pending-session record names a snapshot or layer path outside every registered state root and `oops undo` is invoked
- **THEN** oops refuses to act on that path, deletes nothing, and exits non-zero reporting the corrupted state

#### Scenario: Normal undo
- **WHEN** `oops undo` runs against a valid pending OverlayFS sandbox
- **THEN** the only paths removed are the sandbox's layers and session record, all inside a registered state root; the target is not modified

#### Scenario: Normal undo (snapshot-restore backend)
- **WHEN** `oops undo` runs against a valid pending APFS session whose target still exists as a directory
- **THEN** the target subtree is atomically swapped back to its snapshot state, the displaced tree and session record end up inside a registered state root for deletion, and no path outside the target subtree and the state roots is touched

#### Scenario: Target was replaced by the command
- **WHEN** the wrapped command deleted and recreated the target directory (different inode, same path) and `oops undo` is invoked
- **THEN** the parent identity still verifies, the replacement is treated as part of the command's changes, and the restore proceeds normally

#### Scenario: Target was deleted by the command
- **WHEN** the target path no longer exists and `oops undo` is invoked
- **THEN** the snapshot is renamed into the verified parent at the recorded name, restoring the tree

#### Scenario: Target is a symlink at undo time
- **WHEN** the target path is a symlink and `oops undo` is invoked
- **THEN** undo refuses, modifies nothing, and exits non-zero explaining the symlink hazard

#### Scenario: Parent identity changed since run
- **WHEN** the parent directory of the recorded target no longer matches the recorded device and inode and `oops undo` is invoked
- **THEN** oops refuses the restore, modifies nothing, and exits non-zero explaining the identity mismatch

### Requirement: Single state directory
All persistent oops state (session records, upper layers, work directories,
mount points) MUST live under one well-known directory:
`$XDG_STATE_HOME/oops/`, defaulting to `~/.local/state/oops/`. Deleting that
directory (when nothing is mounted) MUST fully reset oops.

#### Scenario: State is inspectable and nukeable
- **WHEN** a user removes `~/.local/state/oops/` while no sandbox mount is active
- **THEN** oops behaves as if freshly installed, and no oops artifacts remain anywhere else on the filesystem

### Requirement: Destructive tests are container-only
Destructive integration tests MUST create and touch only their own
dedicated temporary trees; they MUST never operate on pre-existing
developer files. In addition, per backend:

- The **OverlayFS suite** (mounts, namespaces) MUST run only inside the
  Linux test container, guarded by the container marker, and MUST be
  skipped elsewhere.
- The **APFS suite** runs on a macOS host (there is no container for
  APFS) behind a **triple gate**, all three required or the tests skip:
  1. an explicit state-root override — every test points
     `XDG_STATE_HOME` at a temp directory it created, so the developer's
     real state root is never involved;
  2. self-created temp directories for every target tree it mutates;
  3. the environment variable `OOPS_TEST_DESTRUCTIVE=1`, set by the
     dedicated make target, never by default.

#### Scenario: Destructive test invoked on the host
- **WHEN** the OverlayFS destructive suite is run outside the container guard (e.g. plain `cargo test` on the dev host)
- **THEN** those tests are skipped or refuse to run; no host paths are touched

#### Scenario: APFS suite without the destructive flag
- **WHEN** the APFS suite runs without `OOPS_TEST_DESTRUCTIVE=1` (e.g. plain `cargo test` on the dev host)
- **THEN** every destructive test skips; no paths are touched

#### Scenario: APFS suite is tempdir-confined
- **WHEN** the APFS destructive suite runs with all three gates satisfied
- **THEN** every path it modifies lies inside its own temporary directories and its test-scoped state root, and the developer's real state root is untouched

### Requirement: Snapshot-restore fine print
For backends using the snapshot-restore model (APFS), user-facing
documentation MUST state, in addition to the existing guarantee boundary:
the command runs against the real files and the guarantee is "always
restorable", not "never touched"; between `run` and `undo`/`commit` the
real tree holds the command's changes, and external observers (cloud sync
clients, file watchers, editors) can see and may propagate that transient
state; changes made inside the target by other processes during that
window are also reverted by undo (collateral undo); a crash inside the
window leaves the tree modified but the snapshot persists and `oops undo`
after restart restores it.

#### Scenario: Fine print is discoverable
- **WHEN** a user reads the README backend matrix or `oops run --help` on macOS
- **THEN** the snapshot-restore model and its exposure-window consequences are stated explicitly

#### Scenario: Crash inside the window is recoverable
- **WHEN** oops or the machine crashes after `oops run` mutated the real tree but before undo/commit, and the user later runs `oops undo`
- **THEN** the target subtree is restored from the persisted snapshot

