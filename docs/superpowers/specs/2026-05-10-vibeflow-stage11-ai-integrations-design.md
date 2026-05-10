# vibeflow — Stage 11: AI integrations (Tier 1 hooks + Tier 3 foreground-process detection)

**Status:** Draft, pending review
**Date:** 2026-05-10
**Author:** brainstormed with Claude

## Summary

Stage 11 ships the project's reason-for-existing: per-tab awareness of when an AI coding tool is waiting on the user. Two of the three integration tiers from the original design spec land in this stage:

- **Tier 1 (Native, Claude Code only):** `integrations/claude-code-hooks.json` — a JSON snippet a user merges into `~/.claude/settings.json`. Maps Claude Code's `Stop` and `UserPromptSubmit` hook events to `vibeflow-emit` invocations that produce OSC 1338 frames. No vibeflow code path changes — Stage 1's protocol crate + Stage 2's dispatcher already consume the bytes correctly.
- **Tier 3 (Heuristic fallback, all tools):** `AiStateTracker.heuristic_active` is currently plumbed but never armed. Stage 11 adds Linux `/proc/<child>/stat` → `tpgid` → `/proc/<tpgid>/comm` foreground-process detection inside `PtySession::tick`, throttled to ~250 ms. When the foreground command name matches a configurable `[ai] tools` allow-list, the existing heuristic-silence timer is armed; output silence then infers `Waiting`.

Tier 2 (wrapper shims `vibeflow-claude` / `vibeflow-codex` / `vibeflow-opencode`) is **out of scope for this stage** — wrapper shims need tool-specific prompt-pattern investigation that hasn't happened yet. Codex / Opencode / Aider / cursor-agent users get Tier 3 awareness in Stage 11; Tier 2 lands in a later stage when we have empirical data on each tool's output.

## Goals & Non-Goals

### Goals

- Claude Code users get a polished out-of-the-box experience: paste-into-settings.json hook, indicator pulses correctly through working → waiting cycles.
- Every tool in the configured `[ai] tools` list gets a heuristic-silence fallback indicator on Linux, even without native or wrapper integration.
- A new `[ai]` config section exposes the four already-existing `TrackerConfig` knobs (`debounce`, `heuristic_silence`, `stale_state`, `tools`) plus the new `foreground_check_interval_ms` to TOML, with hot-reload.
- All proc-reading code is `#[cfg(target_os = "linux")]`-gated; non-Linux builds get a stub returning `None`. v0.1's platform list is Linux-only, but the cross-platform-clean property is preserved.

### Non-Goals (Stage 11)

- Tier 2 wrapper shims (`vibeflow-claude`, `vibeflow-codex`, `vibeflow-opencode`). Deferred until tool-specific output patterns are investigated.
- macOS or Windows process detection. v0.1 platform list is Linux-only; non-Linux gets the stub.
- Process-tree walking (e.g., handling `npm exec claude` by walking up children). Exact basename match against `comm` only; document the limitation.
- Aider Python binding. Deferred to v0.2 per the original design spec.
- Per-tool prompt-pattern detection. Tier 3 only knows "is this process in the AI-tool list" — not "is this AI tool actually waiting right now."
- New protocol additions, new state values. The four existing states (`Active`/`Working`/`Waiting`/`Done`) are sufficient.
- Shell PS1/RPS1 hooks for OSC 133. Already supported by Stage 2's dispatcher; ship as part of a future stage that wires `shells/`.

## Architecture

### Module layout

| File | Status | Responsibility |
|---|---|---|
| `crates/vibeflow/src/session/proc_watch.rs` | NEW | `pub fn foreground_command_name(child_pid: i32) -> Option<String>`. Linux: parses `/proc/<pid>/stat` field 7 (tpgid) via rsplit-`)` algorithm, then reads `/proc/<tpgid>/comm`, trims, returns. Non-Linux: stub returns `None`. Pure logic, no tracker dependency. |
| `crates/vibeflow/src/session/session.rs` | TOUCHED | `PtySession` gains `tools_list: Vec<String>`, `proc_check_interval: Duration`, `last_proc_check: Option<Instant>`. `PtySession::tick(now)` adds throttled call: when `last_proc_check.elapsed() >= proc_check_interval`, call `proc_watch::foreground_command_name`, set `tracker.set_heuristic_active(tools_list.iter().any(|t| t == &name))`, update `last_proc_check`. |
| `crates/vibeflow/src/session/tracker.rs` | TOUCHED (light) | `AiStateTracker::set_config(cfg: TrackerConfig)` added — currently `tracker.config` is read-only after construction. Existing fields (`debounce`, `heuristic_silence`, `stale_state`) and tests stay valid. |
| `crates/vibeflow/src/app.rs` | TOUCHED | Public setters mirror existing `set_default_respect_osc_title` pattern: `set_default_tracker_config`, `set_default_tools_list`, `set_default_proc_check_interval`. `App::new_tab` initializes new fields on each spawned `PtySession` from these defaults. |
| `crates/vibeflow/src/config/schema.rs` | TOUCHED | New `AiSectionSchema` with `Option<…>` fields and `#[serde(deny_unknown_fields)]`. Mirrors Stage 9's `ColorsSection` pattern. |
| `crates/vibeflow/src/config/mod.rs` | TOUCHED | New resolved `AiSection` struct with non-`Option` fields and dark-theme-equivalent dark defaults. `Config::default_values()` populates. `apply_ai_section()` (or extends `apply_*` step) reads the schema, applies, surfaces errors. |
| `crates/vibeflow/src/window.rs::apply_config` | TOUCHED | Constructs `TrackerConfig` from `[ai]` settings, calls `App::set_default_*` setters, walks `app.tabs_mut()` updating each session's `tracker.set_config(...)`, `tools_list`, `proc_check_interval`. |
| `integrations/claude-code-hooks.json` | NEW | Tier 1 ship artifact: 2-hook config (Stop / UserPromptSubmit) calling `vibeflow-emit`. |
| `integrations/README.md` | NEW | ~30 lines: install paths (fresh / merge), `vibeflow-emit` PATH prerequisite, verification steps. |

### Data flow — Tier 3 (heuristic fallback)

```
                                 main thread
                                      │
   winit about_to_wait ─→ App::tick_all(now)
                              │
                              ▼
                    for each PtySession s in app.tabs:
                              │
                              ▼
                    s.tick(now):
                       ├── if last_proc_check.elapsed() >= proc_check_interval:
                       │      ├── name = proc_watch::foreground_command_name(child_pid)
                       │      ├── matched = match name { Some(n) => tools_list.iter().any(|t| t == &n), None => false }
                       │      ├── tracker.set_heuristic_active(matched)
                       │      └── last_proc_check = Some(now)
                       │
                       └── tracker.tick(now)
                              │
                              ▼
                       (existing logic) when heuristic_active && state == Working
                                        && no output for `heuristic_silence`:
                                        transition to Waiting.
```

### Data flow — Tier 1 (Claude Code hooks)

```
   user runs `claude` in a vibeflow tab
                  │
                  ▼
            Claude Code session
              ├── (user submits prompt)
              │     └── UserPromptSubmit hook fires
              │           └── spawns: vibeflow-emit working --tool=claude
              │                 └── writes ESC ] 1338;state=working;tool=claude BEL to stdout
              │                 └── (stdout = same TTY as Claude = vibeflow's PTY)
              │
              ▼
   vibeflow's PTY reader → mpsc → main thread
                  │
                  ▼
   PtySession.process_bytes → OscDispatcher detects 1338 → AiFrame{state=Working,tool=claude}
                  │
                  ▼
   tracker.on_input(AiFrame) → transitions to Working → indicator goes blue
```

Same flow on Stop with `state=waiting`. No new vibeflow code path.

## Components

### `proc_watch::foreground_command_name`

```rust
//! Linux foreground-process detection for Tier 3 heuristic AI-tool awareness.
//! Pure-logic where possible; the I/O paths are gated behind cfg(target_os = "linux").

/// Read /proc/<child_pid>/stat → tpgid → /proc/<tpgid>/comm. Returns the
/// trimmed command name (no parens, no trailing newline) or None on any
/// I/O error or if there's no foreground process group.
///
/// Caveat: kernel truncates comm to 15 chars; match-list entries longer
/// than 15 chars will silently never match.
pub fn foreground_command_name(child_pid: i32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{child_pid}/stat")).ok()?;
        let tpgid = parse_tpgid(&stat)?;
        if tpgid <= 0 {
            return None;
        }
        let comm = std::fs::read_to_string(format!("/proc/{tpgid}/comm")).ok()?;
        Some(comm.trim().to_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = child_pid;
        None
    }
}

/// Parse `tpgid` (field 7 in the canonical proc(5) numbering) from a
/// /proc/<pid>/stat line. The trick: `comm` (field 2) is paren-wrapped
/// and may itself contain `(`, `)`, or whitespace, so split-by-whitespace
/// from the start is wrong. Instead, find the LAST `)` and operate on
/// the suffix; tpgid is then the 6th whitespace-separated token in that
/// suffix (state, ppid, pgrp, session, tty_nr, tpgid).
fn parse_tpgid(stat_line: &str) -> Option<i32> {
    let after_comm = stat_line.rsplit_once(')')?.1.trim_start();
    after_comm.split_whitespace().nth(5)?.parse().ok()
}
```

### `PtySession::tick` modifications

Existing tick body (Stage 1) drives the tracker forward. Stage 11 adds the throttled proc check at the top:

```rust
pub fn tick(&mut self, now: Instant) -> Vec<SessionEvent> {
    // Stage 11: Tier 3 foreground-process check, throttled.
    let due = match self.last_proc_check {
        Some(last) => now.saturating_duration_since(last) >= self.proc_check_interval,
        None => true,
    };
    if due {
        let matched = match crate::session::proc_watch::foreground_command_name(self.child_pid()) {
            Some(name) => self.tools_list.iter().any(|t| t == &name),
            None => false,
        };
        self.tracker.set_heuristic_active(matched);
        self.last_proc_check = Some(now);
    }
    // (existing Stage 1 tracker.tick(...) logic continues here)
    if self.tracker.tick(now) {
        // …existing event push…
    }
    // …
}
```

**`PtySession::child_pid()` does NOT exist yet** — Stage 11 adds a thin accessor:

```rust
// crates/vibeflow/src/session/session.rs
impl PtySession {
    pub(crate) fn child_pid(&self) -> Option<i32> {
        self.child.process_id().map(|p| p as i32)
    }
}
```

`portable_pty::Child::process_id() -> Option<u32>` is verified at `~/.cargo/registry/src/index.crates.io-*/portable-pty-*/src/lib.rs:137`. The wrapper casts to `i32` for /proc paths. Returns `None` if the child has been reaped or never spawned cleanly; `tick`'s proc-check call falls through to `set_heuristic_active(false)`.

### `AiStateTracker::set_config`

```rust
impl AiStateTracker {
    /// Update timing config at runtime. Used by hot-reload.
    /// In-flight timers don't restart; subsequent tick() calls compare
    /// against the new thresholds.
    pub fn set_config(&mut self, config: TrackerConfig) {
        self.config = config;
    }
}
```

### Config schema

`config/schema.rs`:

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSectionSchema {
    pub tools: Option<Vec<String>>,
    pub heuristic_silence_ms: Option<u64>,
    pub stale_state_timeout_s: Option<u64>,
    pub debounce_ms: Option<u64>,
    pub foreground_check_interval_ms: Option<u64>,
}
```

`config/mod.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct AiSection {
    pub tools: Vec<String>,
    pub heuristic_silence_ms: u64,
    pub stale_state_timeout_s: u64,
    pub debounce_ms: u64,
    pub foreground_check_interval_ms: u64,
}

// In Config::default_values():
ai: AiSection {
    tools: vec![
        "claude".into(),
        "codex".into(),
        "opencode".into(),
        "aider".into(),
        "cursor-agent".into(),
    ],
    heuristic_silence_ms: 4000,
    stale_state_timeout_s: 30,
    debounce_ms: 100,
    foreground_check_interval_ms: 250,
}
```

`apply_ai_section` (or equivalent) parses the schema, applies into the resolved struct, errors collected like Stage 9's color parsing.

### `App` setter additions

```rust
impl App {
    pub fn set_default_tracker_config(&mut self, cfg: TrackerConfig) {
        self.tracker_config = cfg;
    }
    pub fn set_default_tools_list(&mut self, tools: Vec<String>) {
        self.default_tools_list = tools;
    }
    pub fn set_default_proc_check_interval(&mut self, interval: Duration) {
        self.default_proc_check_interval = interval;
    }
}
```

`App::new_tab` reads these defaults when constructing each new `PtySession`. Same lifecycle as `default_respect_osc_title` (Stage 9).

### `WindowApp::apply_config`

Extends the existing `apply_config` body with:

```rust
let ai = &config.ai;
let tracker_cfg = TrackerConfig {
    debounce: Duration::from_millis(ai.debounce_ms),
    heuristic_silence: Duration::from_millis(ai.heuristic_silence_ms),
    stale_state: Duration::from_secs(ai.stale_state_timeout_s),
};
let proc_interval = Duration::from_millis(ai.foreground_check_interval_ms);
self.app.set_default_tracker_config(tracker_cfg);
self.app.set_default_tools_list(ai.tools.clone());
self.app.set_default_proc_check_interval(proc_interval);
for s in self.app.tabs_mut().iter_mut() {
    s.tracker.set_config(tracker_cfg);
    s.tools_list = ai.tools.clone();
    s.proc_check_interval = proc_interval;
}
```

## Tier 1 — `integrations/claude-code-hooks.json`

```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "vibeflow-emit waiting --tool=claude"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "vibeflow-emit working --tool=claude"
          }
        ]
      }
    ]
  }
}
```

`integrations/README.md` (~30 lines):
- Two-paragraph intro: what this does, why you'd want it.
- Installation: fresh setup vs merge-into-existing-settings.
- Prerequisite: `vibeflow-emit` on `$PATH` (`cargo install --path crates/vibeflow-protocol`).
- Verify: open vibeflow, run `claude`, send a prompt, observe indicator transitions.
- Pointer: see `docs/protocol.md` if you want to wire other tools.

## Edge cases

- **`/proc` unreadable** (Docker without `--privileged`, restrictive sandbox, non-Linux): `foreground_command_name` returns `None` → `set_heuristic_active(false)` → heuristic timer stays disarmed. Tier 1 still works (doesn't depend on /proc). Logged at `tracing::debug!`, NOT `warn!`.
- **`comm` exceeds 15 chars**: kernel truncates to 15. Tools list entries longer than 15 chars never match. Document in schema comment; default tools list all fit.
- **`comm` contains `(` or `)`**: rsplit-`)` algorithm in `parse_tpgid` handles correctly.
- **Process exits between stat read and comm read**: ENOENT on second read → `None`. Heuristic stays disarmed for one tick; re-checks at next interval.
- **`tpgid <= 0`**: early-return `None`. Defensive against daemon / kernel-thread cases.
- **Tracker config rotation while Working**: `set_config` updates thresholds; in-flight timers compare against new thresholds on next tick. Same hot-reload semantics as Stage 9.
- **Tools list contains `bash` or other shell**: every shell tab matches → false-positive `Waiting` after silence. User config error; documented in schema comment.
- **`vibeflow-emit` not installed**: hooks fail silently per Claude Code's hook execution; user falls back to Tier 3.
- **Apply_config races with tick**: both run on main thread; no concurrency.
- **Spawning new tab post-config-reload**: `App::set_default_*` setters update App's defaults; `App::new_tab` reads them at spawn. Same pattern as `default_respect_osc_title` (Stage 9).
- **Foreground process changes within the 250 ms throttle window**: missed transitions inside the window are smoothed out. Acceptable; humans don't notice 250 ms of indicator latency.

## Testing strategy

### Unit tests (proc_watch.rs)

```rust
#[test]
fn parse_tpgid_handles_simple_case() { … }

#[test]
fn parse_tpgid_handles_paren_in_comm() {
    // Synthetic stat with comm = "(weird)thing"
    let line = "12345 ((weird)thing) S 6789 12345 12345 0 -1 4194304 …";
    assert_eq!(parse_tpgid(line), Some(-1));
}

#[test]
fn parse_tpgid_returns_none_for_malformed() { … }

#[cfg(target_os = "linux")]
#[test]
fn foreground_command_name_round_trips_self() {
    let pid = std::process::id() as i32;
    let name = foreground_command_name(pid);
    // The test process's tpgid likely points at cargo or the test runner;
    // assert non-empty result, don't pin the exact name.
    assert!(name.is_some());
}

#[test]
fn foreground_command_name_returns_none_for_invalid_pid() {
    assert_eq!(foreground_command_name(-1), None);
    assert_eq!(foreground_command_name(i32::MAX), None);
}
```

### Unit tests (tracker.rs — additions to existing block)

```rust
#[test]
fn set_config_updates_heuristic_silence_threshold() {
    let mut t = AiStateTracker::new(TrackerConfig::default());
    let now = Instant::now();
    t.set_heuristic_active(true);
    t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
    t.set_config(TrackerConfig {
        heuristic_silence: Duration::from_millis(1000),
        ..TrackerConfig::default()
    });
    let after_short = t.tick(now + Duration::from_millis(800));
    assert!(!after_short, "should not transition within 1000ms");
    let after_long = t.tick(now + Duration::from_millis(1100));
    assert!(after_long, "should transition past new 1000ms threshold");
    assert_eq!(t.state(), TabState::Waiting);
}
```

### Unit tests (session.rs)

```rust
#[test]
fn tick_throttles_proc_check_to_interval() {
    // Spawn a real PtySession on `bash`; capture initial last_proc_check
    // (None) and proc_check_interval (250ms default).
    let mut s = PtySession::spawn(&["bash"], TrackerConfig::default()).unwrap();
    let t0 = Instant::now();
    s.tick(t0); // first call should trigger proc_watch and set last_proc_check.
    let after_first = s.last_proc_check;
    assert!(after_first.is_some(), "proc check should run on first tick");
    // Tick again 100ms later — within interval. last_proc_check unchanged.
    s.tick(t0 + Duration::from_millis(100));
    assert_eq!(s.last_proc_check, after_first, "proc check should be throttled within interval");
    // Tick at 300ms — past interval. last_proc_check advances.
    s.tick(t0 + Duration::from_millis(300));
    assert!(s.last_proc_check.unwrap() > after_first.unwrap(), "proc check should re-run past interval");
}
```

This requires `PtySession.last_proc_check` and `PtySession.proc_check_interval` to be `pub(crate)` (or accessible via test-only accessors) so the test can read state. Stage 1's pattern in this file uses `pub(crate)` on internal fields used by tests; matching that.

### Unit tests (config/mod.rs)

```rust
#[test]
fn ai_section_defaults_match_spec() {
    let cf = Config::default_values();
    assert_eq!(cf.ai.tools, vec![
        "claude", "codex", "opencode", "aider", "cursor-agent",
    ]);
    assert_eq!(cf.ai.heuristic_silence_ms, 4000);
    assert_eq!(cf.ai.stale_state_timeout_s, 30);
    assert_eq!(cf.ai.debounce_ms, 100);
    assert_eq!(cf.ai.foreground_check_interval_ms, 250);
}

#[test]
fn ai_section_load_from_toml_overrides() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, r#"
[ai]
tools = ["mytool", "claude"]
heuristic_silence_ms = 2500
"#).expect("write");
    let (cf, errors) = Config::load(&path);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(cf.ai.tools, vec!["mytool", "claude"]);
    assert_eq!(cf.ai.heuristic_silence_ms, 2500);
    // Other ai fields keep defaults.
    assert_eq!(cf.ai.stale_state_timeout_s, 30);
}
```

### Integration tests (Linux-only, `crates/vibeflow/tests/ai_integrations.rs`)

```rust
#[cfg(target_os = "linux")]
#[test]
fn tier_3_arms_for_listed_tool() {
    // Spawn a session whose child runs `bash` (so /proc/<bash>/stat → tpgid → comm = "bash").
    // Configure tools = ["bash"] in the test setup.
    // Tick the session; assert tracker.heuristic_active becomes true after one proc_check.
}

#[cfg(target_os = "linux")]
#[test]
fn tier_3_does_not_arm_for_unlisted_tool() {
    // Same spawn, tools = ["claude"] (excludes bash).
    // Tick; assert heuristic_active stays false; no spurious Waiting transition.
}
```

### Manual smoke walk on slmbeast VNC

1. Build release; install `vibeflow-emit` (`cargo install --path crates/vibeflow-protocol`); verify `which vibeflow-emit`.
2. Merge `integrations/claude-code-hooks.json` into `~/.claude/settings.json` (use `jq` one-liner per the README).
3. Open vibeflow, run `claude` in a tab. Send a prompt. Observe: indicator switches to Working (blue) on submit, then Waiting (amber) when Claude finishes — Tier 1 path.
4. Run a long-running shell command (`sleep 30`) in a non-claude tab. Verify heuristic does NOT fire (sleep isn't in tools list) and the tab stays Active.
5. Live-edit config: add `python3` to tools, set `heuristic_silence_ms = 1000`. Reload (just save). Run `python3 -c "import time; time.sleep(30)"` in a tab. After ~1 s of silence, indicator pulses amber — Tier 3 confirmed firing with reloaded thresholds.
6. Kill the python process. Tab returns to Active (no stuck Waiting; the stale-state timer also still works).
7. Inside a Docker container with `--rm`, run vibeflow there if /proc is restricted. Verify Tier 1 hooks still fire (no /proc dependency); Tier 3 silently no-ops; logs at debug level only.
8. With `tools = ["bash"]` configured (intentionally over-broad), open multiple bash tabs and let them sit. Verify multiple tabs pulse amber — confirms config-error footgun is documented behavior.

## Implementation sequencing (rough — refined in plan)

1. `proc_watch::parse_tpgid` + `foreground_command_name` + their unit tests.
2. `AiStateTracker::set_config(cfg)` + test.
3. `[ai]` config schema + resolved struct + defaults + apply step + tests.
4. `App::set_default_tracker_config` / `set_default_tools_list` / `set_default_proc_check_interval` setters; `App::new_tab` reads them.
5. `PtySession` field additions + initialization in `App::new_tab`.
6. `PtySession::tick` throttled proc check + tests.
7. `WindowApp::apply_config` extension to wire `[ai]` into App + per-session updates.
8. `integrations/claude-code-hooks.json` + `integrations/README.md`.
9. Integration tests against real PTY.
10. Senior pre-execution Sonnet review.
11. Manual smoke walk on VNC.
12. Senior holistic Sonnet review at end of stage.

## Risks & mitigations

- **`/proc` parsing footguns** (paren-in-comm, race between stat and comm reads). Mitigated by rsplit-`)` algorithm + None-on-error defensive fallback.
- **15-char comm truncation** silently breaks user expectations. Mitigated by config-comment documentation; default tools list all fit.
- **Hot-reload race** between set_config and tick. Mitigated by main-thread-only execution.
- **Test of `foreground_command_name` round-trip** depends on test environment's tpgid being readable. Could be flaky in unusual sandboxes. Mitigated by asserting only `is_some()`, not the exact name.
- **Tier 1 hook test** can't be fully integration-tested without real Claude Code. The hooks file is shipped as-is and validated by manual smoke walk only.
- **Tier 2 deferral** means Codex / Opencode users get Tier 3 only. Acceptable for v0.1; documented in README; future stage closes the gap.

## Out-of-scope notes for future stages

- **Tier 2 wrapper shims** (`vibeflow-claude`, `vibeflow-codex`, `vibeflow-opencode`): need upstream-tool output investigation. Likely Stage 11.5 or 12.
- **Aider Python binding**: deferred to v0.2 per original design spec.
- **macOS process detection**: requires different API (libproc / proc_pidinfo). Out of v0.1 scope.
- **OSC 133 PS1 hooks** (`shells/vibeflow.{zsh,bash,fish}`): provides Tier-3-equivalent for plain shells. Future stage.
- **Process-tree walking** for `npm exec claude` style invocations: complexity vs benefit unclear; defer until users hit the limitation.
