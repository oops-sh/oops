# Proposal: oops-shell-spike

## Why

A dogfooding run exposed a structural gap in the safety model: an autonomous
agent, told only to use `oops run` and check `oops diff`, finished and then
called `oops commit` itself — closing the safety window before any human
review. The protection model assumes a reviewer sits between `run` and
`commit`/`undo` and enforces nothing. `oops shell` (a per-session sandbox
whose commit/undo decision is taken at exit by the launcher) is the candidate
structural fix. Before an implementation proposal, one question must be
answered honestly and per backend: **does putting an agent inside `oops
shell` actually take the commit/undo decision out of its hands, or does it
just feel like it does?** The two backends do not share a sandbox model, so
the answer may differ — and a safety tool that overstates its guarantee is
actively harmful.

## What Changes

Research spike: no product code, no spec delta. Deliverables:

- `design.md` answering, per backend (Linux/OverlayFS vs macOS/APFS): sandbox
  feasibility for a long-lived interactive shell; where commit/undo authority
  actually sits (structural vs conventional); session lifecycle/orphans;
  nesting; and whether `oops shell` subsumes the macOS stale-cwd finding and
  the prefix-friction motivation. Includes a per-backend verdict table and a
  recommendation with an explicit real-vs-aspirational guarantee split.
- `spikes/oops-shell/` throwaway probes: a macOS authority probe run against
  the shipped 0.2.0 binary (a second, cwd-only process finalizes the
  session — boundary is conventional) and a container-only Linux overlay
  long-lived-shell probe (real lower shadowed inside the namespace — naive
  finalize structurally neutralized, with the mount-privilege escape caveat).

## Capabilities

### New Capabilities

_None — spike only. Any spec revisions are proposed in design.md and enacted
by a future implementation change, if the user approves one._

### Modified Capabilities

_None._

## Impact

- New files under `spikes/oops-shell/` and this change directory. No
  dependencies, no runtime changes, no changes to shipped behavior.
- Output feeds the decision on whether to open an `add-oops-shell`
  implementation proposal, and how honestly to frame the safety claim.

## Non-goals

- Any implementation of `oops shell`, trait/CLI changes, or spec edits.
- The launcher↔shell finalize UX, the confinement layer for a structural
  macOS guarantee, and rootless-Linux hardening (each noted as follow-up in
  design.md).
