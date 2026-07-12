# session — pending-sandbox state between run and undo/commit

## ADDED Requirements

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
