# Proposal: release-v0-1-0

## Why

The core loop works and the diff is readable — time to plant the flag: lock the `oops` crate name on crates.io before someone else takes it, make the repo presentable (README with a real demo), and fix licensing before any external contribution arrives (relicensing later multiplies cost by the number of contributors). Named sessions and the APFS spike are deliberately sequenced after this release.

## What Changes

- **Dual license: MIT OR Apache-2.0.** Add `LICENSE-MIT` and `LICENSE-APACHE`, set `license = "MIT OR Apache-2.0"` in Cargo.toml, update the README license section (currently says MIT only). Rationale: the Rust ecosystem convention — Apache-2.0 adds an explicit patent grant, MIT keeps maximal compatibility.
- **crates.io v0.1.0 publish (name lock).** The `oops` crate name is taken (a stale error-handling crate), so the crate is **`oops-sh`** with `[[bin]] name = "oops"` — you install `oops-sh`, you run `oops`. The README states this mapping explicitly. Version bump 0.0.1 → 0.1.0, complete package metadata (description, `repository`, keywords, categories, `readme`, explicit `include` list so openspec/, docker/, demo/, tests/ stay out of the package), `cargo package` verified, then `cargo publish`. The crate compiles on all platforms and fails closed at runtime outside Linux, which the README/crate description states plainly.
- **Replayable demo recording.** `demo/demo.tape` — a [VHS](https://github.com/charmbracelet/vhs) script that replays the flagship demo (`oops run "rm -rf ./project"` → `oops diff` → `oops undo`) deterministically and renders `demo/demo.gif` (kept under 3 MB, enforced by the make target). A `docker/demo.Dockerfile` layers VHS on top of the existing `oops-dev` image so recording runs inside the sandbox-capable container; `make demo-gif` is the one-command entry point. The GIF is committed and replaces the README placeholder.
- **README release polish.** Demo GIF embedded, install section (`cargo install oops` + the honest "Linux-only in v0.1.0, macOS via container" caveat), license section.
- **Tag `v0.1.0`** and a GitHub release pointing at the crate and the demo.

## Capabilities

### New Capabilities

- `release`: requirements for shipping — dual-license completeness, crates.io package hygiene (what must/must-not be in the package), demo reproducibility (the GIF must be regenerable from the committed tape with one command).

### Modified Capabilities

_None — no runtime behavior changes._

## Impact

- Files: `LICENSE-MIT`, `LICENSE-APACHE`, `Cargo.toml`, `README.md`, `demo/demo.tape`, `demo/demo.gif`, `docker/demo.Dockerfile`, `Makefile`.
- No code changes, no new Rust dependencies. VHS is a build-time tool pulled inside Docker only.
- **Requires you once**: `cargo login` with your crates.io token before the publish step (I cannot and should not hold the token). Publishing is effectively irreversible (versions can be yanked but the name stays ours — which is the point).

## Non-goals

- Named/multiple sessions, APFS spike (explicitly sequenced after v0.1.0).
- CI/CD pipelines, release automation, binary distribution (homebrew/apt) — later.
- Prebuilt binaries; v0.1.0 is `cargo install` only.
- Any change to sandbox/CLI behavior.
