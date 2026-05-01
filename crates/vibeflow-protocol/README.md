# vibeflow-protocol

Reference implementation of **OSC 1338**, vibeflow's open standard for AI-tool state signalling in terminals.

When an AI tool emits an OSC 1338 sequence, a compliant terminal (e.g. [vibeflow](https://github.com/bjhengen/vibeflow)) updates that tab's visual indicator: amber pulse for `waiting`, blue for `working`. The protocol is open so any tool — Rust, JS, shell, Python via stdout — can participate.

This crate is **zero-dependency** and pure-`std`. It also ships a tiny `vibeflow-emit` binary so anything that can `exec` (shell scripts, hooks) can emit too.

## Install

```toml
[dependencies]
vibeflow-protocol = "0.1"
```

## Quick start

```rust
use vibeflow_protocol::{emit, emit_state, Frame, State};

fn main() -> std::io::Result<()> {
    // Simple:
    emit_state(State::Working)?;

    // With detail:
    emit(&Frame::new(State::Waiting)
        .with_tool("claude")
        .with_project("vibeflow"))?;
    Ok(())
}
```

## Parsing

```rust
use vibeflow_protocol::{parse, State};

fn main() {
    let bytes = b"\x1b]1338;state=waiting;tool=claude\x07";
    let frame = parse(bytes).unwrap();
    assert_eq!(frame.state, State::Waiting);
    assert_eq!(frame.tool.as_deref(), Some("claude"));
}
```

## `vibeflow-emit` (CLI)

```bash
$ cargo install vibeflow-protocol
$ vibeflow-emit waiting --tool=claude
$ vibeflow-emit working --tool=codex --project=vibeflow
```

## Wire format

The complete OSC 1338 specification is in [`docs/protocol.md`](https://github.com/bjhengen/vibeflow/blob/main/docs/protocol.md) in the project repository.

## License

Dual-licensed under MIT or Apache-2.0 at your option.
