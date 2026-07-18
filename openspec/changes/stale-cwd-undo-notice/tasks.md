# Tasks: stale-cwd-undo-notice

## 1. Spec + docs

- [ ] 1.1 `safety` spec delta: snapshot-restore undo stale-cwd notice
  (trigger conditions, stderr-only, OverlayFS excluded). **Awaiting review.**
- [ ] 1.2 README: macOS fine print + two-backend guarantee matrix record the
  stale-handle behavior and the Linux/macOS difference.

## 2. Implementation (after approval)

- [ ] 2.1 `src/main.rs` `undo()`: after `restore` + trash succeed, if
  `record.backend == "apfs"` and logical `$PWD` is the recorded target or a
  path beneath it, `eprintln!` the reassurance notice (files safe; stale
  handle; `cd "$PWD"`).
- [ ] 2.2 No notice when `$PWD` unset / outside target; no notice on OverlayFS.

## 3. Tests

- [ ] 3.1 APFS (macOS, destructive-gated): undo with `$PWD` = target →
  reassurance on stderr, stdout unchanged; undo with `$PWD` outside target →
  silent.
- [ ] 3.2 Linux container: undo on OverlayFS → no stale-cwd notice.

## 4. Wrap-up

- [ ] 4.1 Full suite green; `openspec validate --strict`.
