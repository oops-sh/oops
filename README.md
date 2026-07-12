# oops

**Any command can be undone.**

```console
$ oops run "rm -rf ./project"
$ oops undo
# → files are back. oops. 💨
```

oops is a command-line safety layer: it runs any command inside a lightweight
copy-on-write filesystem sandbox, so destructive actions become reversible.
Built for the AI-agent era — coding agents can run at full speed without
permission-prompt fatigue, because every filesystem change they make can be
inspected and rolled back.

<!-- TODO: demo GIF -->

## The four verbs

| Command | What it does |
| --- | --- |
| `oops run "<cmd>"` | Run a command; its filesystem writes land in a sandbox layer, not your files |
| `oops diff` | See what the command created (A), modified (M), deleted (D) |
| `oops undo` | Discard the sandbox — your files were never touched |
| `oops commit` | Apply the sandbox to your real files |

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
```

Work is spec-driven via [OpenSpec](https://github.com/Fission-AI/OpenSpec):
capabilities live in `openspec/specs/`, changes in `openspec/changes/`.

## License

MIT
