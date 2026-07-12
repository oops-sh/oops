# safety — non-negotiable invariants

Every other capability defers to this spec. When a requirement here conflicts
with convenience, performance, or UX anywhere else, this spec wins.

## ADDED Requirements

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

### Requirement: Undo containment
`oops undo` MUST only delete or modify paths inside oops-managed state
directories. It MUST NOT remove, truncate, or rewrite any path outside them,
regardless of session-state corruption or invalid input.

#### Scenario: Corrupted session state points outside the state directory
- **WHEN** the pending-session record names an upper-layer path outside the oops state directory and `oops undo` is invoked
- **THEN** oops refuses to act on that path, deletes nothing, and exits non-zero reporting the corrupted state

#### Scenario: Normal undo
- **WHEN** `oops undo` runs against a valid pending sandbox
- **THEN** the only paths removed are the sandbox's upper layer, work directory, and session record — all inside the oops state directory

### Requirement: Single state directory
All persistent oops state (session records, upper layers, work directories,
mount points) MUST live under one well-known directory:
`$XDG_STATE_HOME/oops/`, defaulting to `~/.local/state/oops/`. Deleting that
directory (when nothing is mounted) MUST fully reset oops.

#### Scenario: State is inspectable and nukeable
- **WHEN** a user removes `~/.local/state/oops/` while no sandbox mount is active
- **THEN** oops behaves as if freshly installed, and no oops artifacts remain anywhere else on the filesystem

### Requirement: Destructive tests are container-only
Integration tests that mount filesystems or exercise destructive commands
MUST be guarded so they only run inside the Linux test container, never
against a developer's host filesystem.

#### Scenario: Destructive test invoked on the host
- **WHEN** the destructive integration test suite is run outside the container guard (e.g. plain `cargo test` on the dev host)
- **THEN** the destructive tests are skipped or refuse to run; no host paths are touched
