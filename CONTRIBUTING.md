# Contributing to vibeflow

Thanks for your interest! vibeflow is a young project and contributions of all
sizes are welcome — bug reports, protocol implementations, docs fixes, and code.

## Quick orientation

- **`crates/vibeflow`** — the terminal emulator (wgpu + winit + alacritty_terminal).
- **`crates/vibeflow-protocol`** — the OSC 1338 protocol library (also published
  to npm from `bindings/npm/`). The protocol spec lives in
  [`docs/protocol.md`](docs/protocol.md).
- Linux (X11/Wayland) is the supported platform for v0.1.

## Building and testing

```sh
cargo build --workspace
cargo test --workspace -- --test-threads=1
```

Note: a handful of PTY/subprocess-detection tests are timing-sensitive and
flake under parallel execution — run the full suite with `--test-threads=1`.

Before pushing, run the same gates CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Manual smoke-test checklists are in [`docs/TESTING.md`](docs/TESTING.md).

## Pull requests

- Open an issue first for anything non-trivial so we can agree on direction.
- Keep PRs focused; include tests for behavior changes.
- CI must be green before merge.

## Implementing OSC 1338 in your tool

You don't need to touch this repo at all — the protocol is an open spec.
Read [`docs/protocol.md`](docs/protocol.md) and emit the escape sequence
directly, or use the [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol)
crate / [npm package](https://www.npmjs.com/package/vibeflow-protocol).
Issues and PRs documenting third-party integrations are very welcome.

## Security

Please report vulnerabilities privately — see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions are dual-licensed under
MIT OR Apache-2.0, the same terms as the project (see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE)).
