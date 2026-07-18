# safety Specification — delta for stale-cwd-undo-notice

## ADDED Requirements

### Requirement: Snapshot-restore undo stale-cwd notice
On a snapshot-restore backend (APFS), `oops undo` restores the target with an
atomic directory swap that replaces the target's inode; a shell whose working
directory is the target (or a directory beneath it) is left holding a stale
directory handle, so subsequent commands in that shell report "No such file
or directory" even though every file was restored. To keep undo from
signalling data loss at the moment it succeeds, after a successful undo oops
MUST, when the backend is snapshot-restore AND the caller's logical working
directory (`$PWD`) is the restored target or a path beneath it, print one
additional notice. The notice MUST state that the files were restored and are
safe, that this is only the shell's stale directory handle, and that
`cd "$PWD"` refreshes it; it MUST NOT state or imply that any data was lost.

The notice MUST be printed to **stderr** (like the undo success message), so
stdout remains a clean machine interface — `undo` has no porcelain mode and
the notice never appears on stdout. It MUST be printed only when both
conditions hold: an unset or out-of-target `$PWD` prints nothing (no false
alarm). Interception backends (OverlayFS) do not replace the target inode and
never produce a stale handle, so on those backends this notice MUST NOT be
printed. This behavior is platform-asymmetric by nature and MUST be documented
as such in user-facing docs.

#### Scenario: APFS undo from inside the target prints the reassurance
- **WHEN** `oops undo` succeeds on the APFS backend and the caller's `$PWD` is the restored target directory (or a subdirectory of it)
- **THEN** oops prints, to stderr after the success message, a notice that the files are safe and that `cd "$PWD"` refreshes the shell's stale directory handle — and stdout is unchanged

#### Scenario: OverlayFS undo prints no such notice
- **WHEN** `oops undo` succeeds on the OverlayFS backend (which does not replace the target inode)
- **THEN** no stale-cwd notice is printed — the shell's cwd is still valid

#### Scenario: APFS undo from outside the target is silent
- **WHEN** `oops undo` succeeds on the APFS backend but the caller's `$PWD` is not the target nor beneath it (e.g. undo run from a parent directory)
- **THEN** no stale-cwd notice is printed
