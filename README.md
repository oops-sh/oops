# oops

**Any command can be undone.**

```console
$ oops run "rm -rf ./project"
$ oops undo
# → files are back. oops. 💨
```

![oops demo: rm -rf sandboxed, diffed, undone](https://raw.githubusercontent.com/oops-sh/oops/main/demo/demo.gif)

oops is a command-line safety layer: it runs any command inside a lightweight
copy-on-write filesystem sandbox, so destructive actions become reversible.
Built for the AI-agent era — coding agents can run at full speed without
permission-prompt fatigue, because every filesystem change they make can be
inspected and rolled back.

## The four verbs

| Command | What it does |
| --- | --- |
| `oops run "<cmd>"` | Run a command; its filesystem writes land in a sandbox layer, not your files |
| `oops diff` | See what the command created (A), modified (M), deleted (D) |
| `oops undo` | Discard the sandbox — your files were never touched |
| `oops commit` | Apply the sandbox to your real files |

### Reading the diff

```console
$ oops run "rm -rf legacy/ && touch NOTES.md"
$ oops diff
Created (1)
  NOTES.md

Deleted (1)
  legacy/

1 created, 1 deleted
```

Colored on a TTY (honors `NO_COLOR`). For scripts and agents,
`oops diff --porcelain` is the **stable** interface: `A/M/D path` lines,
byte-order sorted, a deleted directory as a single `D path/` entry (the
subtree is not expanded), empty output when nothing changed, exit 0 either
way. Known limitation: paths containing newlines can't be represented in
the line-oriented format (`-z` is reserved for a future NUL-terminated
variant).

## Install

```console
$ cargo install oops-sh
```

**The crate is `oops-sh`, the command is `oops`** — the crates.io name
`oops` was already taken, so you install `oops-sh` and run `oops`.

Runtime is **Linux-only** for now (the sandbox is OverlayFS). On any other
platform `oops run` refuses to execute the command rather than run it
unsandboxed. On macOS, use a Linux container or VM — this repo's
`make shell-linux` gives you one.

## Status: Phase 0 (proof of the core loop)

Working today, inside a Linux environment:

- OverlayFS-backed sandbox (`SnapshotBackend` trait; APFS backend planned)
- The full run → diff → undo/commit loop, with integration tests proving the
  flagship demo above restores a byte-identical tree
- `undo` is O(1) — one atomic rename — measured at **< 1ms** on a
  10,000-file tree (target: < 100ms)

## The honest fine print (guarantee boundary)

The sandbox covers **filesystem writes under the directory where you invoked
`oops run`** — nothing else. Not undoable:

- writes outside that tree (`/tmp`, `$HOME`, other mounts)
- network side effects (that email is sent)
- spawned daemons and other process state

Safety invariants (see `openspec/specs/safety/`): if sandbox setup fails,
oops refuses to run the command at all — it never silently falls back to
running unsandboxed; and undo/gc can only ever delete inside oops's own
state directory (`~/.local/state/oops/`).

## Development

The dev host can be macOS; everything that touches OverlayFS runs inside a
privileged Linux container:

```console
make test-linux    # full test suite in the container
make bench-linux   # the undo < 100ms benchmark
make shell-linux   # interactive shell in the test environment
make check         # fast host-side compile check
make demo-gif      # re-render demo/demo.gif from demo/demo.tape (VHS)
```

Work is spec-driven via [OpenSpec](https://github.com/Fission-AI/OpenSpec):
capabilities live in `openspec/specs/`, changes in `openspec/changes/`.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache License 2.0](LICENSE-APACHE), at your option. Unless you explicitly
state otherwise, any contribution intentionally submitted for inclusion in
oops shall be dual-licensed as above, without any additional terms.
