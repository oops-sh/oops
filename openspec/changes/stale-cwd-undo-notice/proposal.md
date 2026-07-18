# Proposal: stale-cwd-undo-notice

## Why

A dogfood session surfaced a real, platform-asymmetric UX defect on the
macOS/APFS backend. After `oops undo`, a shell whose cwd was inside the
protected directory holds a **stale directory handle**: `ls`, `git status`,
etc. print `No such file or directory` even though every file was restored
byte-for-byte. Root cause: APFS undo restores via an atomic directory swap
(`renamex_np(RENAME_SWAP)`); the swapped-out inode is unlinked, and the
shell's cwd fd still points at it. The Linux OverlayFS backend does **not**
hit this — interception undo discards a layer and never replaces the target
inode — so this is a platform-asymmetric, user-visible behavior that strikes
at the worst possible moment: the instant after undo succeeds, it signals the
exact opposite of "your files are safe."

This change is **damage control only**: make undo not scare the user. It does
not attempt to cure the stale handle.

## What Changes

- **Detect and reassure.** After a successful `undo`, when (a) the backend is
  snapshot-restore (APFS) and (b) the caller's logical working directory
  (`$PWD`) is the swapped protected target or a directory beneath it, oops
  prints one extra line after the success message. The line states plainly
  that the files were restored and are safe, that this is only the shell's
  stale directory handle, and that `cd "$PWD"` refreshes it. It MUST NOT imply
  any data loss.
- **Channel / porcelain.** The notice is printed to **stderr**, exactly like
  the existing `undo` success message (`eprintln!` in `main.rs`). `undo` has
  no `--porcelain` mode; keeping the notice on stderr means stdout stays a
  clean machine interface regardless. The spec fixes this as stderr-only.
- **OverlayFS is never affected.** The interception backend does not replace
  the target inode, so the notice MUST NOT be printed there — the spec records
  the asymmetry explicitly and marks OverlayFS out of scope.
- **Docs.** The README macOS fine print and the two-backend guarantee matrix
  record this behavior and the Linux/macOS difference.

## Detection design (for review; not yet implemented)

- Trigger condition, evaluated only after `restore` + trash succeed:
  `record.backend == "apfs"` AND the caller's logical `$PWD` equals
  `record.target` or is a path under it (prefix match on the canonicalized
  recorded target vs. the shell's logical `$PWD`).
- Use the shell's logical `$PWD`, not `getcwd(3)`: after the swap, `getcwd`
  against the stale inode may fail or mislead, whereas `$PWD` reflects where
  the shell *believes* it is — which is exactly the "standing in the swapped
  dir" condition we want to detect. (This mirrors how `pending_session`
  already falls back to `$PWD`.)
- If `$PWD` is unset or outside the target, print nothing (no false alarms).

## Impact

- Affected spec: `safety` (ADDED requirement: snapshot-restore undo stale-cwd
  notice).
- Affected code (implementation, after approval): `src/main.rs` `undo()` — one
  post-success check and `eprintln!`. The `SnapshotBackend` trait, the APFS
  restore path, and the OverlayFS backend are untouched.
- Affected docs: README macOS fine print + guarantee matrix.
- No change to the four verbs' contracts, exit codes, or `diff --porcelain`.

## Non-goals (deferred, evaluated separately)

- **Shell integration** (`oops init zsh`, auto-`cd`/refresh after undo).
- **`oops shell`** sub-shell model.

Both are the *cure* for the stale handle; this change is only the reassurance
so undo does not frighten the user in the meantime.
