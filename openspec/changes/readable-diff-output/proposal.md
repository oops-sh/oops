# Proposal: readable-diff-output

## Why

`oops diff` currently prints raw `A/M/D path` lines — correct, but a human deciding between `undo` and `commit` gets no summary, no visual grouping, and no color. The diff is the product's decision point ("what did that command actually do to my files?"), so it should be readable at a glance. At the same time, agents and scripts need today's stable plain format to keep working.

## What Changes

- `oops diff` default output becomes human-readable: changes grouped into **Created / Modified / Deleted** sections with per-section counts, colored when stdout is a TTY (green/yellow/red), ending with a one-line summary (`3 created, 1 modified, 2 deleted`).
- New flag `oops diff --porcelain`: exactly the current format (`A/M/D path`, sorted, one per line, no color, empty output for no changes) — the stable machine interface, guaranteed not to change.
- Color rules: ANSI colors only when stdout is a TTY; the `NO_COLOR` environment variable disables color; `--porcelain` never colors.
- Empty diff in human mode prints `no changes` instead of nothing.
- No new dependencies: hand-rolled ANSI codes (a few constants) and `std::io::IsTerminal` for TTY detection.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `diff`: the "Change classification" requirement currently mandates plain `A/M/D` lines as the only output; it is modified to make that the `--porcelain` contract, and new requirements are added for the human-readable default (sections, counts, summary) and color behavior.
- `cli`: the four-verb surface gains its first flag (`diff --porcelain`); the exit-code requirement is unchanged.

## Impact

- Code: `src/diff.rs` (renderers), `src/main.rs` (flag + TTY detection). The backend and session layers are untouched — this is rendering only.
- No new crates. `std::io::IsTerminal` is stable std.
- Compatibility: anything parsing today's output must switch to `--porcelain`; nothing ships yet that depends on it, so now is the cheapest moment to make the default human-first.

## Non-goals

- Content diffs (line-level file diffs) — this is path classification rendering only.
- Pager integration, `--stat` histograms, JSON output (future proposals if needed).
- A `--color=always/never` override flag — TTY detection + `NO_COLOR` covers Phase 1; forcing color can come later if agents ask for it.
