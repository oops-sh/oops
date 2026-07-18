# session Specification — delta for rootless-linux-followups

## ADDED Requirements

### Requirement: Rootless trash reclamation
A rootless OverlayFS mount leaves a `work/work` directory owned by the mapped
uid with mode `000`; the plain unprivileged user cannot delete it (it cannot
enter a mode-000 directory even as the owner without first changing the mode,
which recursive deletion does not do). Therefore gc of a registered state
root's `trash/` MUST run inside an **identity-mapped user namespace** — one
that maps the invoking uid to user-namespace root — so it holds the
`CAP_DAC_OVERRIDE` needed to reclaim those leftovers. Reclamation MUST remain
best-effort and fail-safe: if the user namespace cannot be entered, gc still
runs and reclaims whatever is deletable without it, and an unreclaimed trash
entry MUST never affect correctness — only disk usage (per undo's
rename-then-async-delete policy). Rootless-produced trash MUST eventually be
reclaimed so the state roots do not grow unboundedly across runs.

Because gc now runs with `CAP_DAC_OVERRIDE` inside that user namespace, the
incidental "not enough permission" that previously kept it from following an
agent-planted symlink out of the tree is GONE — trash content is written by
the sandboxed (adversary) command, so containment is now the ONLY defense and
MUST be enforced exactly as commit replay enforces it: every path is reached
by an `O_NOFOLLOW` component walk anchored on a registered state root,
deletion is `unlinkat`, a directory is entered only via `openat(O_NOFOLLOW)`,
and a symlink component is unlinked as a link and never traversed. The
containment anchor — opening each `trash/` from its registered root — MUST be
established BEFORE any privilege elevation; elevation only serves to enter the
mode-000 directories, and every operation under it stays relative to the
anchored fds with no path re-parsed. A symlink an agent leaves in trash can
therefore only be deleted, never followed.

#### Scenario: gc does not follow an out-of-tree symlink in trash
- **WHEN** the trashed session contains a symlink (planted by the sandboxed command) pointing at a path outside the state root, and gc reclaims trash with `CAP_DAC_OVERRIDE`
- **THEN** gc unlinks the symlink without traversing it, and the out-of-tree target is byte-identical afterward

#### Scenario: Rootless trash is reclaimed
- **WHEN** a rootless `oops run` is undone — leaving a mode-000 `work/work` in the trashed session directory — and gc sweeps (a background `__gc` and/or a later sweep)
- **THEN** the trashed session directory, including the mode-000 work directory, is removed and the state root's `trash/` becomes empty

#### Scenario: The plain user can reset state after gc
- **WHEN** the unprivileged user removes the oops state directory after gc has run
- **THEN** no permission-protected rootless leftovers remain to block the removal, and oops behaves as freshly installed

#### Scenario: Reclamation is fail-safe when the userns is unavailable
- **WHEN** gc cannot enter an identity-mapped user namespace (e.g. userns restricted)
- **THEN** gc still runs, reclaims whatever is deletable, and no oops command misbehaves — only some trash may linger for a later sweep
