# Design: readable-diff-output

## Context

`src/diff.rs` has one `render()` producing `A/M/D path` lines from
`Vec<Change>`; `main.rs::diff_cmd` prints it. Rendering is fully decoupled
from the backend, so this change touches only those two files.

## Goals / Non-Goals

**Goals:** human-first default output, a frozen `--porcelain` machine
format, zero new dependencies.

**Non-Goals:** content-level diffs, pagers, JSON, `--color` override flags.

## Decisions

### D1: Two renderers, one data source

`diff.rs` gains `render_porcelain(&[Change])` (today's `render`, renamed)
and `render_human(&[Change], color: bool)`. `main.rs` picks based on the
flag. No renderer inspects the filesystem — same `Vec<Change>` input.

### D2: No color crate

Four ANSI constants (green, yellow, red, reset) hand-rolled in `diff.rs`.
Color eligibility computed in `main.rs` as
`stdout.is_terminal() && env NO_COLOR unset && !porcelain` using
`std::io::IsTerminal`. Alternative (owo-colors/anstyle) rejected: a
dependency for four escape codes, and the working agreement prices new
dependencies deliberately.

### D3: Porcelain is a frozen contract

The porcelain format is documented in the spec as MUST-NOT-change. Human
output carries no stability promise — it can be improved freely later
(this is the reason to ship the split now, before anyone scripts against
the default).

## Risks / Trade-offs

- [Someone already scripts against the current default] → nothing has
  shipped outside this repo; the integration tests are updated in the same
  change.
- [Human format churn breaks screenshots/docs] → acceptable; only porcelain
  is contractual.

## Open Questions

None.
