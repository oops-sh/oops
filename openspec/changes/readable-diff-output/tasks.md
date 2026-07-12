# Tasks: readable-diff-output

## 1. Rendering

- [ ] 1.1 Split `diff.rs` into `render_porcelain` (today's format, renamed) and `render_human` (Created/Modified/Deleted sections with counts, one-line summary, `no changes` for empty), with host-runnable unit tests for both including empty and single-section cases
- [ ] 1.2 Add color support to `render_human` (green/yellow/red ANSI constants, `color: bool` parameter) with unit tests asserting exact escape sequences on and off

## 2. CLI wiring

- [ ] 2.1 Add `--porcelain` to the `Diff` subcommand; compute color eligibility (`IsTerminal` + `NO_COLOR` + porcelain) in `main.rs`; update the existing container integration tests to use `--porcelain` where they assert exact lines
- [ ] 2.2 New container integration tests: human-mode sections + summary (piped, so no color), `NO_COLOR`/pipe produce no escape codes, empty diff prints `no changes` (human) and nothing (porcelain)

## 3. Docs

- [ ] 3.1 README: show human output in the demo section, document `--porcelain` as the stable interface for scripts/agents
