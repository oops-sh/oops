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

#### Scenario: Rootless trash is reclaimed
- **WHEN** a rootless `oops run` is undone — leaving a mode-000 `work/work` in the trashed session directory — and gc sweeps (a background `__gc` and/or a later sweep)
- **THEN** the trashed session directory, including the mode-000 work directory, is removed and the state root's `trash/` becomes empty

#### Scenario: The plain user can reset state after gc
- **WHEN** the unprivileged user removes the oops state directory after gc has run
- **THEN** no permission-protected rootless leftovers remain to block the removal, and oops behaves as freshly installed

#### Scenario: Reclamation is fail-safe when the userns is unavailable
- **WHEN** gc cannot enter an identity-mapped user namespace (e.g. userns restricted)
- **THEN** gc still runs, reclaims whatever is deletable, and no oops command misbehaves — only some trash may linger for a later sweep
