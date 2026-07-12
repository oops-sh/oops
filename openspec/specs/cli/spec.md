# cli Specification

## Purpose
TBD - created by archiving change add-core-sandbox-loop. Update Purpose after archive.
## Requirements
### Requirement: Four-verb surface
The `oops` binary MUST expose exactly four subcommands in Phase 0:
`run <command>`, `diff`, `undo`, `commit`. `run` takes the command as a
single shell-string argument. No other user-facing subcommands SHALL exist
(internal/hidden helpers for re-exec inside the namespace are permitted but
MUST NOT appear in help output).

#### Scenario: Help lists the four verbs
- **WHEN** `oops --help` is invoked
- **THEN** run, diff, undo, and commit are listed, and no other public subcommand appears

### Requirement: Exit codes
`oops run` MUST exit with the wrapped command's exit status on success of
the sandbox machinery. oops-level failures (sandbox setup, no pending
session, corrupted state) MUST exit non-zero with a distinct error message
on stderr, and MUST be distinguishable from the wrapped command's own
failure by message.

#### Scenario: Wrapped command's status is propagated
- **WHEN** `oops run "exit 7"` completes with the sandbox working correctly
- **THEN** oops exits with status 7

#### Scenario: diff/undo/commit with nothing pending
- **WHEN** `oops diff`, `oops undo`, or `oops commit` is invoked with no pending sandbox
- **THEN** oops exits non-zero and prints a clear "no pending sandbox" message

