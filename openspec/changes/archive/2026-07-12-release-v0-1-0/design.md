# Design: release-v0-1-0

## Context

No runtime code changes; this is packaging, licensing, and presentation.
The only moving part worth designing is the demo recording pipeline.

## Goals / Non-Goals

**Goals:** name locked on crates.io, clean dual licensing, a README a
stranger understands in 30 seconds, a demo GIF anyone can regenerate.

**Non-Goals:** CI, binary distribution, release automation.

## Decisions

### D1: Demo pipeline — VHS layered on the dev image

`docker/demo.Dockerfile`: `FROM oops-dev` + install VHS (and its ttyd/ffmpeg
deps) from Charm's apt repo. `make demo-gif`:

1. build the release binary in the container (reusing the cargo volumes),
2. build the demo image,
3. `docker run --privileged --tmpfs /root/.local/state/oops` the demo image
   running `vhs demo/demo.tape`, mounting the repo for tape + GIF output.

The tape scripts a fixed 1200x600 terminal, sets up a `project/` directory,
then: `oops run "rm -rf ./project"` → `ls` (still there... gone in sandbox
view? no — the real view, files intact) → `oops diff` → `oops undo` → `ls`.
Alternative considered: asciinema + agg — two tools and a cast intermediate;
VHS is one declarative file, which is exactly "replayable script".

### D2: GIF is committed

`demo/demo.gif` lives in the repo (~a few hundred KB): README on GitHub and
crates.io must render without a build step. Regeneration is `make demo-gif`;
the tape is the source of truth, the GIF is a build artifact we happen to
commit.

### D3: Package `include` allowlist

`include = ["src/**", "README.md", "LICENSE-*"]` in Cargo.toml — allowlist
over `exclude` denylist so future repo additions (specs, tapes, CI) can
never leak into the package by default.

## Risks / Trade-offs

- [VHS rendering differs across versions → GIF churn] → pin the VHS version
  in demo.Dockerfile; the GIF only changes when we change the tape.
- [crates.io README renders the GIF via absolute URL] → use the
  raw.githubusercontent.com URL in the README image tag so it renders on
  both GitHub and crates.io.
- [`cargo publish` needs the user's token] → user-gated task; everything up
  to `cargo publish --dry-run` is verifiable without it.

## Open Questions

None.
