# vibeflow — Design Spec

**Status:** Draft, pending review
**Date:** 2026-05-01
**Author:** brainstormed with Claude

## Summary

vibeflow is a from-scratch GPU-accelerated terminal emulator for Linux, written in Rust, designed around a single thesis: **a terminal should know — and show, at a glance — when an AI tool is waiting on the user vs. working on a task.** The flagship feature is per-tab state awareness driven by a small open standard (`OSC 1338`), implemented via a tiered integration strategy so every popular AI coding tool (Claude Code, Codex, Opencode, Aider, …) gets some level of support out of the box. The terminal also delivers iTerm-grade tab/color polish that's been missing on Linux. Distributed as a single static binary; published as a public GitHub project for the broader vibe-coding community.

## Goals & Non-Goals

### Goals (v0.1)

- A daily-driver Linux terminal: PTY + shell, full ANSI/VT, copy-paste, scrollback, font shaping (CJK, emoji, ligatures), GPU rendering at native frame rate.
- Single window, multiple tabs. Two-line tab format: title (line 1) + running process or AI subtitle (line 2).
- "Notice"-style state indicator on each tab: 3px left-edge stripe + tinted subtitle text. Amber pulse when waiting on user; steady blue when working; muted gray when idle.
- Open standard `OSC 1338` for AI-tool state signaling. Rust crate (`vibeflow-protocol`), npm package (`@vibeflow/protocol`), and shell helper (`vibeflow-emit`) ship as polyglot bindings.
- Three-tier AI integration: native (protocol), wrapper (shipped shims), heuristic (process-name + idle-output detection).
- TOML configuration with hot-reload.
- X11 + Wayland on Linux.

### Non-Goals (v0.1, deferred)

- Splits / panes within a tab.
- In-buffer search.
- Mac and Windows packaged builds (the codebase will be cross-platform-clean since `winit + wgpu + portable-pty` already abstract this; just unpackaged).
- Image protocols (kitty graphics, sixel).
- Headless GPU snapshot tests in CI.
- Plugin / scripting layer beyond TOML config.
- Telemetry, crash reporting, auto-update.
- Python binding for `vibeflow-protocol` (added in v0.2 if Aider integration needs it).

## Differentiator

The "Notice" indicator and the OSC 1338 protocol are the project's unique value. Color semantics, constant across the product:

| State | Color | Meaning |
|---|---|---|
| `waiting` | amber, soft pulse on stripe | AI tool or process is waiting for user input |
| `working` | blue, steady stripe + tinted subtitle | AI tool or process is actively running |
| `idle` | muted gray | shell at prompt, nothing running |
| `active` | no special styling | the focused tab; default everywhere else |

The pulse animation runs only while at least one tab is in `waiting` state. Idle terminals don't repaint at 60 fps.

## Architecture

### High-level

```
External (in PTY child process)
  AI tools (Claude Code, Codex, Opencode, Aider, ...)
    └─ via vibeflow-protocol bindings → OSC 1338 ; state=… ; tool=… ; project=…
  User shell (zsh / bash / fish)
    └─ via shipped PS1 hook → standard OSC 133 prompt markers
        ↓ stdout bytes
─────────────── vibeflow binary ───────────────
  Per-tab PtySession:
    PTY child  →  OscDispatcher  →  alacritty_terminal grid
                       └────────→  AiStateTracker
  Global:
    WindowManager (winit)  ·  TabBar Renderer  ·  Grid Renderer (wgpu)
    ConfigLoader (TOML, hot-reload)  ·  InputRouter
        ↓ wgpu draw calls
─────────────── Output ───────────────
  OS window (X11 / Wayland)
```

Bytes flow downward (PTY child → grid + tracker → renderers → window). Input flows upward (window events → router → focused PTY child stdin).

### Process & threading model

Single GUI process. Two thread classes:
- **Main thread:** winit event loop, wgpu rendering, all `App` state mutation, config hot-reload application. Required to be single-threaded by winit/wgpu.
- **Reader threads (one per PTY):** blocking `read()` on the PTY master fd. On bytes available, sends them through an `mpsc::Sender<Bytes>` to the main thread. Joined when the tab closes.

No tokio in v0.1 — `std::thread + std::sync::mpsc` matches what alacritty does and is the standard Rust 101 concurrency primitive. The choice can be revisited if a future feature genuinely needs an async runtime.

`AiStateTracker` is mutated only on the main thread; reader threads never touch it. No `Arc<Mutex<…>>` needed for state.

## Components

### Workspace layout

```
vibeflow/
├── Cargo.toml                         # workspace manifest
├── crates/
│   ├── vibeflow/                      # binary crate
│   │   └── src/
│   │       ├── main.rs                # ~50 LOC
│   │       ├── app.rs                 # ~120 LOC
│   │       ├── window.rs              # ~180 LOC
│   │       ├── render/{grid,tabs,font}.rs   # ~400 + 220 + 150 LOC
│   │       ├── session/{mod,pty,osc,tracker}.rs  # ~120 + 80 + 100 + 120 LOC
│   │       ├── input.rs               # ~120 LOC
│   │       ├── config.rs              # ~140 LOC
│   │       └── theme.rs               # ~80 LOC
│   └── vibeflow-protocol/             # library crate, zero deps
│       └── src/lib.rs                 # ~150 LOC
├── bindings/
│   └── npm/                           # @vibeflow/protocol — ~30 LOC, zero deps
│                                      # (vibeflow-emit lives inside the vibeflow-protocol crate
│                                      #  as a small bin target — see crates/vibeflow-protocol/src/bin/)
├── shells/                            # opt-in user shell hooks
│   ├── vibeflow.zsh
│   ├── vibeflow.bash
│   └── vibeflow.fish
├── integrations/
│   └── claude-code-hooks.json         # snippet to add to ~/.claude/settings.json
├── docs/
│   ├── protocol.md                    # OSC 1338 spec — the open standard
│   └── TESTING.md                     # manual smoke checklist
├── README.md
├── LICENSE-MIT
└── LICENSE-APACHE
```

Estimated ~1,900 LOC of vibeflow code on top of leveraged libraries.

### Key types

| Type | Location | Role |
|---|---|---|
| `App` | `app.rs` | Owns `Vec<PtySession>`, active tab index, config handle. Single-threaded "central authority." |
| `PtySession` | `session/mod.rs` | One per tab. Owns the child PTY, reader thread join handle, alacritty grid, `AiStateTracker`. Communicates with `App` via mpsc channel. |
| `OscDispatcher` | `session/osc.rs` | Streaming state machine. Watches for `ESC ]`. Identifies `OSC 133` and `OSC 1338` and consumes those bytes; passes everything else through to the alacritty grid unchanged. |
| `AiStateTracker` | `session/tracker.rs` | Per-session state machine `idle ↔ working ↔ waiting ↔ active`. Driven by OSC events. Debounces flapping (<100 ms transitions ignored). Two timeout fallbacks: (a) the **heuristic-output-silence** timeout (default 4000 ms) used by Tier 3 fallback to infer `waiting` from observed quiet on a known AI process; (b) a **stale-state** timeout (default 30 s) that resets a session to `active` if a tool emits a state but never updates again — protects against stuck indicators when a tool dies mid-task. |
| `State` | `vibeflow-protocol/src/lib.rs` | Public enum used by both vibeflow and external AI tools. The contract. |
| `Config` / `Theme` | `config.rs`, `theme.rs` | Serde-derived structs from TOML. Hot-reloaded via `notify` crate; new config sent to renderers via channel. |

### Dependencies (locked-in set)

```toml
# vibeflow (binary)
winit = "0.30"               # windowing — X11 + Wayland
wgpu = "0.20"                # GPU rendering
alacritty_terminal = "0.24"  # VT/ANSI parsing + grid + scrollback
portable-pty = "0.8"         # PTY spawn (cross-platform-clean)
cosmic-text = "0.12"         # font shaping (emoji, fallback, ligatures)
serde = { version = "1", features = ["derive"] }
toml = "0.8"
notify = "6"                 # config file watcher
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"

# vibeflow-protocol (library)
# zero deps. only std.
```

Versions will drift before ship; this list defines the locked-in dependency set as of design time.

## Data Flow

### Flow A — "Claude emits 'waiting'" (the headline scenario)

1. Claude Code calls `vibeflow_protocol::emit_state(State::Waiting)`.
2. The library writes `\x1b]1338;state=waiting;tool=claude\x07` to stdout.
3. PTY master fd becomes readable; reader thread wakes from `read()`.
4. Reader thread copies bytes into a buffer and sends them via `mpsc::Sender<Bytes>` to the main thread.
5. `App::poll()` drains the channel and routes to the correct `PtySession::process_bytes(&buf)`.
6. Session feeds bytes to `OscDispatcher`, byte-by-byte.
7. Dispatcher detects `ESC ]`, buffers until `BEL` or `ST`, parses params:
    - 7a — Recognises `1338`: bytes are *consumed*, parsed `Frame { state: Waiting, tool: "claude", project: None }` is applied to `AiStateTracker`.
    - 7b — Other bytes are forwarded to `alacritty_terminal::Term::input()` as normal.
8. `AiStateTracker` transitions `working → waiting`; sets `last_change = now()`.
9. `App` calls `winit.request_redraw()` since something changed.
10. Next frame: `TabBarRenderer` reads `tracker.state` for each tab; draws amber stripe + schedules pulse animation. Renderer keeps requesting redraws while any tab is in `waiting`.

### Flow B — User keystroke

`winit::KeyboardInput` → `App` → focused `PtySession::send_input(bytes)` → write to PTY master fd → child sees it on stdin.

### Flow C — Plain shell, no AI

Shell emits `OSC 133;A` (prompt-start) then `OSC 133;B` (prompt-end). Dispatcher consumes both; tracker → `idle`. Command starts (`OSC 133;C`) → `working`. Command ends (`OSC 133;D`) → `idle`. Same dispatcher, same tracker, no AI tool needed. Requires opt-in PS1/RPS1 hook from `shells/`.

## Protocol — OSC 1338

### Wire format

```
ESC ] 1338 ; key=value [; key=value ]* (BEL | ST)
```

- `ESC` is `0x1B`. `BEL` is `0x07`. `ST` (string terminator) is `ESC \`.
- Keys (UTF-8, alphanumeric + `_`):
    - `state` — required. One of `waiting`, `working`, `done`, `active`. Unknown values → ignored, logged at debug level.
    - `tool` — optional. Free-form string (e.g., `claude`, `codex`, `aider`). Used for display and grouping.
    - `project` — optional. Free-form string. Surfaces in tab subtitle when present.
- Values: percent-encoded if they contain `;`, `=`, control chars, or non-ASCII. Decoded by parser.
- Maximum total sequence length: **4 KiB** (enforced by dispatcher; over-long sequences are dropped on the floor and parsing resumes at the next `ESC`).
- Stability: this format is the public contract. Additions (new keys, new states) must be backwards-compatible — old consumers ignore unknown keys.

### State enum (canonical)

```rust
pub enum State {
    Active,    // default; nothing special
    Working,   // tool is running / generating
    Waiting,   // tool is waiting for user input — the headline state
    Done,      // tool just finished a task; transient — usually flips back to active
}
```

### Polyglot bindings (v0.1)

| Binding | Audience | Surface |
|---|---|---|
| `vibeflow-protocol` (Rust crate, crates.io) | Rust AI tools, vibeflow itself | `emit_state`, `emit`, `parse`, `State`, `Frame` |
| `@vibeflow/protocol` (npm) | TypeScript/Node tools (Claude Code, Codex CLI, Opencode) | same surface, ~30 LOC, zero deps |
| `vibeflow-emit` (tiny Rust binary, built from the protocol crate) | shell scripts, hooks, anything that can `exec` | `vibeflow-emit waiting --tool=claude` writes the bytes to stdout. Single-file, statically linked, ships in the same release artifacts as `vibeflow`. |

Python binding deferred to v0.2.

### Three-tier integration strategy

1. **Native (Tier 1):** Tool calls a vibeflow-protocol binding directly. Fastest, most accurate.
    - **Claude Code:** integration shipped in `integrations/claude-code-hooks.json` — user pastes into `~/.claude/settings.json`. `Stop` hook emits `state=waiting`; `UserPromptSubmit` emits `state=working`. No Claude Code source changes required.
    - **Aider:** PR upstream to add `vibeflow-protocol` Python binding (in v0.2).
    - **Codex / Opencode:** propose PRs upstream once the protocol has traction; in the meantime, Tier 2 covers it.
2. **Wrapper (Tier 2):** vibeflow ships small wrappers (`vibeflow-claude`, `vibeflow-codex`, `vibeflow-opencode`) that the user aliases their commands to. Each spawns the real tool as a subprocess, watches output for known prompt patterns, and emits OSC 1338 on the tool's behalf.
3. **Heuristic fallback (Tier 3):** Inside vibeflow. If no native or wrapper signal AND foreground process matches a configured AI-tool list (`claude`, `codex`, `opencode`, `aider`, …) AND output stream has been silent for longer than `heuristic_silence_ms` (default 4000 ms, configurable) → infer `waiting`. Rapid output → `working`. Imperfect but ensures every AI tool gets some awareness.

The README sells Tier 1; Tier 3 ensures day-1 use never feels broken.

## Configuration

`~/.config/vibeflow/config.toml`. Hot-reloaded via the `notify` crate. Every documented setting has a sensible default; the file is optional.

### Schema (v0.1)

```toml
[window]
font_family = "JetBrains Mono"
font_size = 13.0
opacity = 1.0
padding_px = 8

[theme]
# Named theme or full color spec
preset = "vibeflow-dark"   # or omit and define [theme.colors] yourself

[theme.indicator]
waiting_color = "#ffbd2e"
working_color = "#5fb4ff"
idle_color    = "#45454f"
pulse = true

[ai]
tools = ["claude", "codex", "opencode", "aider", "cursor-agent"]
heuristic_silence_ms = 4000   # Tier 3: infer `waiting` after this much output silence
stale_state_timeout_s = 30    # reset to `active` if a tool emits state but never updates again
debounce_ms = 100             # ignore state transitions closer together than this

[tabs]
position = "top"      # "top" | "bottom"
two_line = true
show_subtitle = true
default_title_from = "cwd"   # "cwd" | "process" | "auto"

[keybindings]
new_tab = "Ctrl+Shift+T"
close_tab = "Ctrl+Shift+W"
next_tab = "Ctrl+Tab"
prev_tab = "Ctrl+Shift+Tab"
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"
reopen_dead_tab = "Ctrl+Shift+R"
```

## Visual Design

- **Tab format:** two lines. Line 1 = title (default = cwd basename, overridable per-tab). Line 2 = subtitle (`<tool> · <state>` for AI tabs; running command name for shells; empty for an idle prompt). Subtitle truncates with ellipsis if narrower than tab.
- **Notice indicator:** 3px stripe along the **left** edge of each tab. Color from state. Stripe alpha pulses on a 1.4s sine when state is `waiting`. Other states are steady.
- **Subtitle tint:** subtitle text color follows the same state color (more saturated on the active tab, muted on inactive). Reinforces the stripe in peripheral vision.
- **Active tab:** background slightly lighter; subtitle slightly more saturated; stripe unchanged.
- **Default theme:** dark, neutral background `#0e0e12`; tab bar `#15151c`. Restraint over flash.
- **Default font:** "JetBrains Mono" 13pt with cosmic-text fallback chain → system monospace if missing.

## Error Handling

### Principles

1. **Fail loud at startup, fail soft at runtime.** GPU init errors are fatal with actionable messages. Misbehaving tabs at runtime are isolated to that tab; the window survives.
2. **A failing tab never kills sibling tabs.** Each `PtySession` is the unit of failure. Dead tabs show an in-tab error banner with reason and the `Ctrl+Shift+R` retry shortcut.

### Failure modes

| Failure | Where | Response | User experience |
|---|---|---|---|
| Config TOML malformed | startup or hot-reload | Log warning; fall back to last-good config (or built-in defaults at startup) | Banner: "config invalid, using defaults" |
| GPU/wgpu init fails | startup | **Fatal.** Print actionable message; suggest `VIBEFLOW_BACKEND=gl` env override | Stderr message + non-zero exit |
| Font missing | renderer | cosmic-text fallback chain → system default | Possibly less-pretty glyphs; no crash |
| PTY spawn fails | tab open | Tab opens to in-tab error pane | "shell `/bin/zsh` not found. Press `Ctrl+Shift+R` to retry." |
| Child process exits | runtime | Mark session dead; freeze grid as last known state | "zsh exited 0. Press `Ctrl+Shift+R` to reopen." (red styling for non-zero exit) |
| Reader thread errors | runtime | Treated as child exit | Same as above |
| Malformed OSC 1338 | dispatcher | Ignore; log at debug; restart parser at next `ESC` | Invisible (intended) |
| Truncated OSC (no terminator) | dispatcher | 4 KiB cap; on overflow drop, resume at next `ESC` | Invisible. Caps documented in protocol spec. |
| State flapping from a misbehaving tool | tracker | Debounce: ignore transitions <100 ms apart | Indicator stays calm; abusive tools can't strobe |
| `alacritty_terminal` panic | session input | `std::panic::catch_unwind` around input call; mark tab dead with diagnostic | Tab shows panic info; sibling tabs unaffected |
| GPU surface lost | renderer | Detect `SurfaceError::Lost`, recreate | Brief flicker; no crash |
| Hot-reload reads partial file | config watcher | Try-parse; retry once after 200 ms; if still bad, keep old config | Usually invisible |

### Logging

- `tracing` crate. Default `INFO` to `stderr` and `~/.local/state/vibeflow/vibeflow.log` (rotated at 10 MB, last 3 retained).
- `RUST_LOG=vibeflow=debug` for verbose.
- Structured log line per state transition.

### What we explicitly do NOT do

- No telemetry / crash reporter.
- No auto-restart of dead tabs (user decides).
- No silent error swallowing.

## Testing

### Tiers

| Tier | What | Tool |
|---|---|---|
| Unit (must-have v0.1) | `vibeflow-protocol::parse`/`emit` round-trip; `OscDispatcher`; `AiStateTracker` transitions; `Config` parsing | `cargo test` + `proptest` |
| Fuzz (must-have v0.1) | `parse(arbitrary bytes)` never panics; `OscDispatcher::feed(arbitrary bytes)` never panics or OOMs | `cargo-fuzz` |
| Integration (nice-to-have v0.1) | Fake child PTY emits known sequence → assert tracker state; config hot-reload | `cargo test --test integration` + `portable-pty` |
| Smoke (manual, v0.1) | Pre-release checklist | `docs/TESTING.md` |
| Snapshot GPU (v0.2+) | Render headless, compare PNGs | wgpu headless + `insta` |

### Manual smoke checklist (`docs/TESTING.md`)

1. Open vibeflow; verify dark theme renders, font correct, cursor blinks.
2. Open three tabs; verify two-line tabs show right subtitle (zsh, bash, claude).
3. Run `claude` in a tab; verify state goes `working` (blue) on prompt submit, `waiting` (amber pulse) on Claude return, `idle` after timeout.
4. Run `vim` / `htop` / `less` / `fzf` — verify keys, mouse selection, scroll all work.
5. `cat` a 10MB log file — smooth scroll, no UI hitches.
6. Kill a tab's child; verify dead-tab banner; `Ctrl+Shift+R` reopens.
7. Edit `config.toml` while running; hot-reload visible (e.g., theme color change).
8. Resize / minimize / monitor switch; no GPU surface errors.

### CI (GitHub Actions, v0.1)

- Linux only: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, fuzz smoke (60 s per target).
- Mac/Windows CI deferred to v0.2+.

## Distribution & Licensing

- **License:** dual MIT / Apache-2.0 (Rust convention; permissive for community adoption).
- **Crate publishing:** `vibeflow-protocol` published to crates.io from v0.1. `vibeflow` binary published to crates.io as well, but the primary install is via prebuilt binaries.
- **npm:** `@vibeflow/protocol` published from v0.1.
- **Prebuilt binaries:** GitHub Releases — `vibeflow-x86_64-linux-gnu.tar.xz` (and `aarch64-linux-gnu` if cross-compile is straightforward).
- **Distros:** AUR + Homebrew tap considered for v0.2 once ABI/feature surface stabilises.

## Out of Scope (v0.1)

- Splits / panes within a tab.
- In-buffer search.
- Mac and Windows packaged builds.
- Image protocols.
- Plugin / scripting layer.
- Telemetry, crash reporting, auto-update.
- Python binding for `vibeflow-protocol`.
- Headless GPU snapshot tests.
- Cross-driver GPU compatibility matrix.
- Performance regression CI.

## Open Questions

None outstanding. All settled during the brainstorm.

(One housekeeping item to handle on first commit: the working directory is currently `~/dev/ai_term/`. Whether to rename to `~/dev/vibeflow/` is purely cosmetic — the repo, crate, and binary are all `vibeflow` regardless. User to decide.)

## Estimated Effort

Rough sizing for v0.1 from a Rust-newcomer pace. Subject to revision when the implementation plan is written.

| Area | Estimate |
|---|---|
| `vibeflow-protocol` crate + tests + fuzz harness + npm binding + shell helper | 1 week |
| `OscDispatcher` + `AiStateTracker` + tests + fuzz harness | 1 week |
| `PtySession` + reader thread + `App` glue | 1 week |
| `winit + wgpu` initial render plumbing | 1–2 weeks |
| Grid renderer (cells, cursor, colors, attributes, selection) | 2–3 weeks |
| Tab bar renderer (two-line, Notice indicator, pulse animation) | 1 week |
| Font atlas + cosmic-text shaping | 1 week |
| Input routing (keys, mouse, copy/paste, scroll) | 1 week |
| Config + hot-reload + theme | 3–4 days |
| Shell hooks (zsh/bash/fish) + Claude Code hooks | 2 days |
| README + protocol.md + TESTING.md + LICENSE | 2 days |
| Bring-up, polish, smoke fixes, release prep | 1 week |

**Total v0.1: ~3 months of evening/weekend work, paced for learning Rust along the way.**
