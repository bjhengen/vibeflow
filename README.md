# vibeflow

A GPU-accelerated terminal emulator for Linux that knows when your AI tool is waiting on you.

> **Status:** Pre-alpha. The `vibeflow-protocol` foundation lands first; the terminal itself follows.

## Repository layout

- [`crates/vibeflow-protocol/`](crates/vibeflow-protocol/) — Rust reference implementation, published as [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol) on crates.io. Includes the `vibeflow-emit` CLI.
- [`bindings/npm/`](bindings/npm/) — TypeScript reference implementation, published as [`@vibeflow/protocol`](https://www.npmjs.com/package/@vibeflow/protocol) on npm.
- [`docs/protocol.md`](docs/protocol.md) — the canonical OSC 1338 wire-format specification.
- [`docs/superpowers/specs/`](docs/superpowers/specs/) — design specs.
- [`docs/superpowers/plans/`](docs/superpowers/plans/) — implementation plans.

The terminal binary (`crates/vibeflow/`) is not yet built; this repository currently ships only the protocol foundation.

## License

Dual-licensed under either of:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
