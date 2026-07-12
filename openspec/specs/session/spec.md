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
Session records MUST be stored as human-readable JSON under the oops state
directory, keyed by target directory. Records MUST contain only paths inside
the state directory as deletable state (per safety's undo containment).

#### Scenario: Session record location
- **WHEN** `oops run "true"` completes in directory D
- **THEN** a JSON session record for D exists under the oops state directory and names D, the upper/work layer paths, and the command

### Requirement: Undo is rename-then-async-delete
`undo`'s synchronous work MUST be O(1) in the size of the change set: one
atomic same-filesystem rename of the session directory into
`<state>/trash/`, after which undo reports success. Deletion of trash
contents happens asynchronously (background process and/or later gc sweep).
An undeleted trash entry MUST never affect correctness — only disk usage.

#### Scenario: Undo of a huge change set
- **WHEN** `oops undo` runs against a sandbox whose upper layer contains tens of thousands of entries
- **THEN** undo returns success in under 100ms, a new `oops run` works immediately, and the old layers are eventually removed from `trash/`

#### Scenario: Background deletion dies
- **WHEN** the asynchronous deletion is killed before finishing
- **THEN** the leftover trash entry is removed by a later gc sweep, and no oops command misbehaves in the meantime

### Requirement: Orphaned-state gc
Every `oops run` MUST sweep the state directory before creating its session:
delete entries under `<state>/trash/`, and move session directories lacking
a valid `session.json` into `trash/`. gc MUST NOT touch validly pending
sessions, and all gc deletions are subject to the safety spec's undo
containment (state-directory paths only).

#### Scenario: Orphan from a crashed run
- **WHEN** a session directory without a parseable record exists and `oops run "true"` is invoked
- **THEN** the orphan is swept into trash and the new run proceeds normally

#### Scenario: gc leaves pending sessions alone
- **WHEN** a valid pending session for another directory exists during a gc sweep
- **THEN** that session and its layers are untouched

### Requirement: Stale session detection
If a session record exists but its mount is no longer active (e.g. after a
reboot or container restart), `diff`, `undo`, and `commit` MUST detect this
and either recover safely (undo: discard the layers) or refuse with a clear
message (commit: refuse rather than merge a possibly-incomplete layer).

#### Scenario: Undo after the mount disappeared
- **WHEN** the sandbox mount is gone but the session record and upper layer remain, and `oops undo` is invoked
- **THEN** oops discards the leftover layers and record, restoring the "nothing pending" state

#### Scenario: Commit after the mount disappeared
- **WHEN** the sandbox mount is gone and `oops commit` is invoked
- **THEN** oops refuses to merge and exits non-zero explaining the sandbox is stale

