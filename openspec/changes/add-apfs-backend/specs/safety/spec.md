# safety — delta for add-apfs-backend

> **REVIEW FLAG (逐字審查):** the two MODIFIED requirements below — "Undo
> containment" and "Destructive tests are container-only" — change the
> project's core invariants. Please review them word by word; the rest of
> this delta is additive fine print.

## MODIFIED Requirements

### Requirement: Undo containment
`oops undo` MUST modify exactly two things and nothing else:

1. **The session's protected target subtree** — restoring it, atomically,
   to its recorded snapshot state. Before any restore operation, the target
   MUST be re-verified against the identity recorded at `run` time
   (canonical path plus `st_dev` and `st_ino`); on mismatch, undo refuses
   and modifies nothing.
2. **Registered oops state roots** — deletions and renames by undo
   (including its asynchronous deletion phase) and by gc are permitted only
   for paths that a containment check proves to be inside one of the
   registered state roots (the primary root and any per-volume roots listed
   in the registry).

Regardless of session-state corruption or invalid input, undo and gc MUST
NOT remove, truncate, rewrite, or swap any other path on the system.
Backends that never modify the target during undo (e.g. OverlayFS, which
discards a layer) automatically satisfy clause 1 by doing nothing to the
target.

#### Scenario: Corrupted session state points outside the state roots
- **WHEN** the pending-session record names a snapshot or layer path outside every registered state root and `oops undo` is invoked
- **THEN** oops refuses to act on that path, deletes nothing, and exits non-zero reporting the corrupted state

#### Scenario: Normal undo (interception backend)
- **WHEN** `oops undo` runs against a valid pending OverlayFS sandbox
- **THEN** the only paths removed are the sandbox's layers and session record, all inside a registered state root; the target is not modified

#### Scenario: Normal undo (snapshot-restore backend)
- **WHEN** `oops undo` runs against a valid pending APFS session
- **THEN** the target subtree is atomically restored to its snapshot state, the displaced tree and session record end up inside a registered state root for deletion, and no path outside the target subtree and the state roots is touched

#### Scenario: Target identity changed since run
- **WHEN** the directory at the recorded target path no longer matches the recorded device and inode (e.g. it was deleted and recreated) and `oops undo` is invoked
- **THEN** oops refuses the restore, modifies nothing, and exits non-zero explaining the identity mismatch

### Requirement: Destructive tests are container-only
Destructive integration tests MUST create and touch only their own
dedicated temporary trees; they MUST never operate on pre-existing
developer files. In addition, per backend:

- The **OverlayFS suite** (mounts, namespaces) MUST run only inside the
  Linux test container, guarded by the container marker, and MUST be
  skipped elsewhere.
- The **APFS suite** runs on a macOS host (there is no container for APFS)
  but MUST confine every path it creates, mutates, restores, or deletes to
  freshly created temporary directories and a test-scoped state root
  (`XDG_STATE_HOME` pointed at a temp dir), never the developer's real
  state root or working files.

#### Scenario: Destructive test invoked on the host
- **WHEN** the OverlayFS destructive suite is run outside the container guard (e.g. plain `cargo test` on the dev host)
- **THEN** those tests are skipped or refuse to run; no host paths are touched

#### Scenario: APFS suite is tempdir-confined
- **WHEN** the APFS destructive suite runs on a macOS host
- **THEN** every path it modifies lies inside its own temporary directories and its test-scoped state root, and the developer's real state root is untouched

## ADDED Requirements

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
