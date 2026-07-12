# release Specification

## Purpose
TBD - created by archiving change release-v0-1-0. Update Purpose after archive.
## Requirements
### Requirement: Dual licensing
The project SHALL be licensed `MIT OR Apache-2.0`. The repository MUST
contain both full license texts (`LICENSE-MIT`, `LICENSE-APACHE`), the
Cargo.toml `license` field MUST read `MIT OR Apache-2.0`, and the README
MUST state the dual license. All three locations MUST agree.

#### Scenario: License consistency
- **WHEN** the license is inspected in Cargo.toml, the LICENSE files, and the README
- **THEN** all state MIT OR Apache-2.0 with both full texts present

### Requirement: Crate and command naming
The crates.io crate is `oops-sh` (the name `oops` is squatted by an
unmaintained crate); the installed binary is `oops` via
`[[bin]] name = "oops"`. This mapping MUST be stated in the README install
section and in the crate description, and MUST be consistent across
Cargo.toml (`name`, `[[bin]]`), the README, and the GitHub repository
metadata (`repository` field pointing at oops-sh/oops).

#### Scenario: Install-name mapping is documented
- **WHEN** a user reads the README install section or the crates.io page
- **THEN** it is explicit that `cargo install oops-sh` installs a binary invoked as `oops`

### Requirement: crates.io package hygiene
The published crate MUST contain only what an installer needs: `src/`,
`Cargo.toml`, `README.md`, and both LICENSE files. `openspec/`, `docker/`,
`demo/`, `tests/`, and `Makefile` MUST NOT be in the package (enforced via
an explicit `include` list). Package metadata MUST include `description`,
`repository`, `readme`, `keywords`, and `categories`. `cargo package` MUST
succeed, and the package MUST build on non-Linux platforms (where the
binary fails closed at runtime, as the crate description states).

#### Scenario: Package contents
- **WHEN** `cargo package --list` runs
- **THEN** only src/, Cargo.toml (+generated variants), README.md, and the two LICENSE files are listed

### Requirement: Reproducible demo recording
The README demo GIF MUST be regenerable with one command (`make demo-gif`)
from a committed VHS tape script that replays the flagship demo inside the
sandbox-capable container. The tape MUST be deterministic: fixed terminal
size, fixed theme, no wall-clock-dependent output in frame content. The
generated GIF MUST be at most 3 MB, enforced by the make target.

#### Scenario: Regenerating the GIF
- **WHEN** `make demo-gif` runs on a machine with Docker
- **THEN** `demo/demo.gif` is produced from `demo/demo.tape` showing run → diff → undo with the files restored, and the target fails if the GIF exceeds 3 MB

### Requirement: Version tags match the crate
Every crates.io publish MUST correspond to a git tag `v<version>` on the
commit that was packaged, with a GitHub release.

#### Scenario: v0.1.0 traceability
- **WHEN** oops 0.1.0 exists on crates.io
- **THEN** tag `v0.1.0` exists on the packaged commit with a GitHub release

