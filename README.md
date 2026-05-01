# vibeflow

A GPU-accelerated terminal emulator for Linux that knows when your AI tool is waiting on you.

> **Status:** Pre-alpha. The `vibeflow-protocol` foundation lands first; the terminal itself follows.

## Repository layout

- `crates/vibeflow-protocol/` — the open-standard OSC 1338 protocol library + `vibeflow-emit` CLI. Published to crates.io.
- `bindings/npm/` — `@vibeflow/protocol`, the TypeScript sibling. Published to npm.
- `docs/protocol.md` — the canonical OSC 1338 wire-format specification.
- `docs/superpowers/specs/` — design specs.
- `docs/superpowers/plans/` — implementation plans.

## License

Dual-licensed under either of:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
