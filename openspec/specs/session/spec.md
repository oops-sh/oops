# session Specification

## Purpose
TBD - created by archiving change add-core-sandbox-loop. Update Purpose after archive.
## Requirements
### Requirement: Single pending sandbox
Phase 0 supports exactly one pending sandbox per target directory. After
`oops run` completes, a session record MUST exist describing the pending
sandbox (target directory, layer paths, wrapped command, timestamp, command
exit status). `undo` and `commit` MUST consume it; a second `run` while one
is pending MUST be refused with a message pointing at `diff`/`undo`/`commit`.

#### Scenario: Run while a sandbox is pending
- **WHEN** `oops run "true"` is invoked in a directory that already has a pending sandbox
- **THEN** oops refuses, exits non-zero, and tells the user to undo or commit first

#### Scenario: Undo consumes the session
- **WHEN** `oops undo` succeeds
- **THEN** the session record and sandbox layers are gone, and a subsequent `oops undo` reports "no pending sandbox"

### Requirement: Durable, inspectable session records
Session records MUST be stored as human-readable JSON under a registered
oops state root, keyed by target directory. Records MUST name the backend
that created them, the target's canonical path, the target parent
directory's identity (`st_dev`, `st_ino`), and MUST contain only paths
inside registered state roots as
deletable state (per safety's undo containment; the target path itself is
restorable, not deletable).

#### Scenario: Session record location
- **WHEN** `oops run "true"` completes in directory D
- **THEN** a JSON session record for D exists under a registered state root and names D, the backend, the layer/snapshot paths, and the command

### Requirement: Undo is rename-then-async-delete
`undo`'s synchronous work MUST be O(1) in the size of the change set: a
constant number of atomic same-filesystem operations — for interception
backends, one rename of the session directory into the state root's
`trash/`; for snapshot-restore backends, the identity-checked swap followed
by that rename — after which undo reports success. The swap deposits the
displaced tree inside the session directory, so the same rename carries it
into `trash/`, where it is deleted like any other trash content. Deletion
of trash contents happens asynchronously (background process and/or later
gc sweep). An undeleted trash entry MUST never affect correctness — only
disk usage.

#### Scenario: Undo of a huge change set
- **WHEN** `oops undo` runs against a sandbox whose change set contains tens of thousands of entries
- **THEN** undo returns success in under 100ms, a new `oops run` works immediately, and the old layers are eventually removed from `trash/`

#### Scenario: Background deletion dies
- **WHEN** the asynchronous deletion is killed before finishing
- **THEN** the leftover trash entry is removed by a later gc sweep, and no oops command misbehaves in the meantime

#### Scenario: Displaced tree from a snapshot-restore undo is reclaimed
- **WHEN** `oops undo` restores an APFS session via the atomic swap, displacing the command's tree into the session directory
- **THEN** the displaced tree lands in the state root's `trash/` and is removed by the asynchronous deletion or a later gc sweep — it does not accumulate across undos

### Requirement: Orphaned-state gc
Every `oops run` MUST sweep oops state before creating its session: delete
entries under each registered, currently-mounted state root's `trash/`, and
move session directories lacking a valid `session.json` into that root's
`trash/`. Roots whose volume is not mounted MUST be skipped (never
created, never errored on). gc MUST NOT touch validly pending sessions,
and every gc deletion is subject to the safety spec's undo containment
(the path must prove to be inside one of the registered roots).

#### Scenario: Orphan from a crashed run
- **WHEN** a session directory without a parseable record exists in any mounted state root and `oops run "true"` is invoked
- **THEN** the orphan is swept into that root's trash and the new run proceeds normally

#### Scenario: gc leaves pending sessions alone
- **WHEN** a valid pending session for another directory exists during a gc sweep
- **THEN** that session and its layers are untouched

#### Scenario: Unmounted volume root is skipped
- **WHEN** the registry lists a state root on a volume that is not currently mounted and gc runs
- **THEN** gc skips it without error and without creating any path under the mount point

### Requirement: Stale session detection
If a session record exists but its backing sandbox state is unusable —
for OverlayFS, the layers vanished (e.g. tmpfs cleared by a reboot); for
APFS, the snapshot directory is missing — `diff`, `undo`, and `commit`
MUST detect this and either recover safely (undo: discard what remains; on
APFS a missing snapshot means undo MUST refuse rather than guess) or
refuse with a clear message (commit: refuse rather than act on incomplete
state). APFS sessions persist on disk and MUST remain fully usable across
reboots — a pending session after a crash is recovered with a normal
`oops undo`.

#### Scenario: Undo after the mount disappeared
- **WHEN** the OverlayFS sandbox mount is gone but the session record and upper layer remain, and `oops undo` is invoked
- **THEN** oops discards the leftover layers and record, restoring the "nothing pending" state

#### Scenario: Commit after the mount disappeared
- **WHEN** the OverlayFS sandbox mount is gone and `oops commit` is invoked
- **THEN** oops refuses to merge and exits non-zero explaining the sandbox is stale

#### Scenario: APFS session survives a reboot
- **WHEN** a pending apfs session exists, the machine reboots, and `oops undo` is invoked in the target directory
- **THEN** the target is restored from the persisted snapshot exactly as if no reboot had happened

#### Scenario: APFS snapshot missing
- **WHEN** an apfs session record exists but its snapshot directory is gone, and `oops undo` or `oops commit` is invoked
- **THEN** undo refuses to restore (nothing to restore from) and reports the stale session; commit refuses likewise; the record can only be cleared explicitly

### Requirement: Per-volume state roots
The primary state root remains `$XDG_STATE_HOME/oops` (default
`~/.local/state/oops`). Because snapshot-restore requires the snapshot and
target on the same volume, a target on a different volume than the primary
root uses a per-volume state root at `<volume-mount>/.oops/state/` with the
identical layout (`sessions/`, `trash/`). Every per-volume root MUST be
recorded, at creation time, in a registry (`volumes.json`) under the
primary root; registry writes MUST be atomic (write to a temporary file,
then rename into place), so a crash can never leave a truncated registry.
Containment checks and gc operate over exactly the registered set. If the per-volume root cannot be created (read-only
volume, permissions), `run` MUST refuse (fail closed). Deleting a volume's
`.oops/` (plus the registry entry) MUST fully reset oops for that volume.

#### Scenario: Target on a secondary volume
- **WHEN** `oops run "true"` executes with a target on a mounted APFS volume other than the home volume
- **THEN** the session and snapshot are created under that volume's `.oops/state/` root, and the root is listed in the primary registry

#### Scenario: Read-only volume fails closed
- **WHEN** the target's volume cannot host a state root (e.g. read-only)
- **THEN** the command is never executed and oops exits non-zero explaining why

### Requirement: Session lookup by logical path fallback
Sessions are looked up by the canonicalized working directory. If the
working directory cannot be resolved or matches no session — the expected
state after the wrapped command deleted or replaced the target directory
itself — lookup MUST fall back to the shell's logical `$PWD`, compared
against recorded target paths. This is what makes the deleted-target and
symlink restore branches reachable from a shell still sitting in the
damaged location.

`$PWD` is a lookup hint, never an authority: lookup (by either key) only
selects which pending session record to act on. The path that undo
restores is always the record's canonical target, and the safety spec's
identity verification — the parent `st_dev`/`st_ino` check and the three
restore branches — MUST execute unconditionally, regardless of how the
session was found. A forged or stale `$PWD` can therefore at most select
a session that would restore its own recorded target; it can never
redirect a restore.

#### Scenario: Undo from a deleted cwd
- **WHEN** the wrapped command deleted the target directory and the user runs `oops undo` from a shell whose `$PWD` still names it
- **THEN** the session is found via the `$PWD` fallback and the restore proceeds per the safety spec's branches

#### Scenario: $PWD cannot redirect a restore
- **WHEN** `oops undo` finds a session via the `$PWD` fallback
- **THEN** the restore acts on the record's canonical target, with the parent identity check and the restore-branch checks applied exactly as for a canonical-cwd lookup

