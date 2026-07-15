# `oops shell` spike — authority-boundary probes

Throwaway probes for the `oops shell` research spike. They establish, per
backend, whether putting an agent *inside* a sandboxed session actually
takes the commit/undo decision out of its hands. Conclusions and the full
per-backend verdict live in
`openspec/changes/oops-shell-spike/design.md`.

## macOS — `authority_probe_macos.sh`

Runs the shipped `oops` (0.2.0+) on a self-created temp target with a temp
`XDG_STATE_HOME` (never the real state root). It shows that a **second,
independent process** — given only the target directory, no session
id — finalizes the session the first process created:

```
== process #2 (independent: no handle from process #1, only the cwd) ==
oops: committed `oops run "..."` to the real files.
== result ==
COMMITTED by process #2 — the unreviewed change is now permanent.
```

Verdict: on snapshot-restore, finalize authority is **being in the target
dir**. No capability, token, or namespace gates it. An in-shell agent holds
exactly that position, so the macOS boundary is **conventional**.

Run: `bash spikes/oops-shell/authority_probe_macos.sh` (macOS, APFS host).

## Linux — `overlay_shell_probe_linux.sh`

Container-only (needs `CLONE_NEWNS` + overlay mount privilege, like the
OverlayFS destructive suite). Hosts a scripted "session" of many commands
inside one unshared overlay mount and then inspects the real lower tree:

- **A.** a long-lived shell inside the overlay works; all writes land in the
  upper layer.
- **B1.** after the namespace exits, the real lower tree is **pristine** —
  the merge (upper → real lower) is unreachable from inside because the
  lower is shadowed by the overlay at the target path. The naive finalize is
  structurally neutralized.
- **B2.** escape-hatch caveat: a process with mount privilege in the
  namespace can `umount -l` the overlay and reach the lower directly. The
  structural block holds against a cooperative agent and against `oops
  commit`, **not** against an adversary with `CAP_SYS_ADMIN` in the
  namespace — the fix is confinement (rootless userns / dropped caps).

Run inside the repo's privileged Linux container:
`make shell-linux` then `OOPS_IN_CONTAINER=1 bash
spikes/oops-shell/overlay_shell_probe_linux.sh`.

> Not executed on the spike author's host (macOS, no Docker available). The
> Linux structural claim in design.md is grounded in the current
> `src/backend/overlayfs.rs` mount layout (overlay mounted *over* the target,
> so `lowerdir == target` is shadowed inside the namespace); this probe is
> the reproducible check.
