# Tasks: release-v0-1-0

## 1. Licensing

- [ ] 1.1 Add LICENSE-MIT and LICENSE-APACHE (full texts), set `license = "MIT OR Apache-2.0"` in Cargo.toml, update the README license section

## 2. Packaging

- [ ] 2.1 Cargo.toml release metadata: version 0.1.0, description mentioning the Linux-only runtime, keywords, categories, `readme`, `include` allowlist; verify with `cargo package --list` and `cargo publish --dry-run`

## 3. Demo recording

- [ ] 3.1 `demo/demo.tape` (VHS, pinned version, fixed geometry) replaying the flagship demo; `docker/demo.Dockerfile` (FROM oops-dev + VHS); `make demo-gif` target
- [ ] 3.2 Generate `demo/demo.gif`, commit it, embed in README via the raw.githubusercontent URL

## 4. README release polish

- [ ] 4.1 Install section (`cargo install oops`, Linux-only caveat with the container escape hatch), GIF at the top, license section

## 5. Ship (user-gated)

- [ ] 5.1 User runs `cargo login`; then `cargo publish`, tag `v0.1.0`, create the GitHub release linking crate + demo
