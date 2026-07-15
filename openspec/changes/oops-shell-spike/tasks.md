# Tasks: oops-shell-spike

## 1. Probes

- [x] 1.1 `spikes/oops-shell/authority_probe_macos.sh`: with the shipped
  `oops` binary on a temp target + temp `XDG_STATE_HOME`, show a second
  independent process finalizing a session with only the cwd (macOS boundary
  is conventional). Executed on the dev host.
- [x] 1.2 `spikes/oops-shell/overlay_shell_probe_linux.sh`: container-only
  probe of a long-lived shell inside an unshared overlay mount, showing the
  real lower is shadowed (naive finalize neutralized) plus the mount-privilege
  escape caveat. Authored; runs in the repo's privileged Linux container (not
  executed on the macOS host).

## 2. Design answers

- [x] 2.1 Q1: backend feasibility — Linux long-lived shell as namespace
  payload (clean); macOS has only snapshot-restore-per-session (crux), with
  the extended-exposure-window fine print.
- [x] 2.2 Q2: authority boundary made concrete — session keyed by target dir,
  no token; Linux structural (shadowed lower) vs macOS conventional; the
  honesty split (real vs aspirational) and the marker soft-guard.
- [x] 2.3 Q3: session lifecycle & orphans across crash/kill/reboot per
  backend; existing gc/stale-session model extends; abandoned-session gap.
- [x] 2.4 Q4: nesting — forbid, fail closed, justified per backend.
- [x] 2.5 Q5: subsumption — inner stale-cwd gone, launcher stale-cwd remains
  (macOS); prefix friction fully removed.
- [x] 2.6 Per-backend verdict table + recommendation with the one-sentence
  headline answer up front.

## 3. Wrap-up

- [ ] 3.1 Spike reviewed by the user; findings decide whether to open an
  `add-oops-shell` implementation proposal and how to frame the safety claim.
