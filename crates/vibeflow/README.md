# vibeflow

> A GPU-accelerated Linux terminal emulator that knows when your AI tool is waiting on you.

## Install

```sh
cargo install vibeflow
```

Or download a single-file AppImage from the [GitHub Releases](https://github.com/bjhengen/vibeflow/releases).

## Quick start

Launch it: `vibeflow`. Open a new tab with `Ctrl+Shift+T`. The per-tab indicator stripe shows the AI-state for that tab:

- **Blue** — AI tool is working
- **Amber (pulsing)** — AI tool is waiting on you
- **Neutral** — plain shell / no AI state

State arrives via the **OSC 1338 protocol** (an open standard for AI-tool state signalling) — see the [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol) crate. AI tools emit one byte sequence per state transition; vibeflow renders the cue.

### Make AI-state work with Claude Code

Add these hooks to `~/.claude/settings.json` so Claude Code reports its state to vibeflow:

```json
{
  "hooks": {
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "PreToolUse":       [{"matcher": "*", "hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "PostToolUse":      [{"matcher": "*", "hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "Stop":             [{"hooks": [{"type": "command", "command": "vibeflow-emit waiting --tool=claude"}]}],
    "Notification":     [{"hooks": [{"type": "command", "command": "vibeflow-emit waiting --tool=claude"}]}]
  }
}
```

`vibeflow-emit` ships with the `vibeflow-protocol` crate (`cargo install vibeflow-protocol`).

## License

Dual-licensed under either of MIT or Apache-2.0 at your option. See the [GitHub repository](https://github.com/bjhengen/vibeflow) for the full README, configuration reference, themes, and protocol documentation.
