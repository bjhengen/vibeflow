# vibeflow integrations

This directory ships drop-in integration files for popular AI coding tools.

## Claude Code (`claude-code-hooks.json`)

The **five-hook** set that emits OSC 1338 frames so vibeflow's tab indicator
tracks Claude Code through `working → waiting` cycles:

- `UserPromptSubmit` → `working` (Claude is processing your prompt)
- `PreToolUse` → `working` (a tool call is starting)
- `PostToolUse` → `working` (the tool returned; Claude is still working)
- `Stop` → `waiting` (Claude finished and is waiting for your next prompt)
- `Notification` → `waiting` (Claude is asking for input)

> All five matter. With fewer (e.g. only `UserPromptSubmit` + `Stop`), nothing
> re-emits `working` after a tool round, so the tab shows a spurious `waiting`
> (amber) flicker mid-turn while Claude is actually working. This is a Claude
> Code hook-coverage semantic, not a vibeflow bug.

### Prerequisites

`vibeflow-emit` must be on your `$PATH`. From crates.io:

```bash
cargo install vibeflow-protocol
```

(Or, from a checkout of this repo: `cargo install --path crates/vibeflow-protocol`.)
Either installs `vibeflow-emit` to `~/.cargo/bin/vibeflow-emit`. Verify:

```bash
which vibeflow-emit
vibeflow-emit waiting --tool=claude   # should emit a single OSC 1338 frame
```

### Installation

#### Path A — fresh setup

If you don't have a `~/.claude/settings.json` yet, just copy:

```bash
mkdir -p ~/.claude
cp claude-code-hooks.json ~/.claude/settings.json
```

#### Path B — merge into existing settings

If you already have `~/.claude/settings.json`, merge the `hooks` block in. Using `jq`:

```bash
jq -s '.[0] * .[1]' ~/.claude/settings.json claude-code-hooks.json > /tmp/merged.json
mv /tmp/merged.json ~/.claude/settings.json
```

Or open the file in your editor and paste the `hooks` object under the
top-level object.

### Verify

1. Open vibeflow.
2. Run `claude` in a tab.
3. Send a prompt. Tab indicator switches to working (blue).
4. Wait for Claude to finish. Tab indicator switches to waiting (amber, pulses).

If the indicator doesn't change, run `vibeflow-emit waiting --tool=claude`
in a vibeflow tab manually — it should emit OSC bytes that immediately
flip the indicator to waiting. If THAT works but Claude's hooks don't,
verify the JSON is correctly structured and `which vibeflow-emit` resolves.

## Other tools

Codex CLI, Opencode, Aider, and other AI tools without native hook support
fall back to vibeflow's Tier 3 heuristic: when the foreground process name
is in the `[ai] tools` list (default: `claude codex opencode aider cursor-agent`)
AND output has been silent for 4 s during a `Working` state, the indicator
infers `Waiting`. Crude but works.

Native integrations for Codex / Opencode are tracked for a future stage.
Aider Python binding is deferred to v0.2.

See [`docs/protocol.md`](../docs/protocol.md) for the OSC 1338
protocol specification if you want to wire other tools yourself.
