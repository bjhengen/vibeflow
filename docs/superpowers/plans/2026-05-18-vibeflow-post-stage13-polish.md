# vibeflow post-Stage-13 polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land three pre-existing fixes — theme-driven wgpu clear color, a one-shot first-redraw grid-size reconcile, and a unified explicit-stale tracker recovery (single `[ai] explicit_stale_state_s` knob).

**Architecture:** All changes are additive and confined. T1 reuses the existing `active_theme_colors` in `render/mod.rs`. T2 follows the established `[ai]` config-field pattern (schema → resolved `Ai` → `TrackerConfig` → hot-reload). T3 adds one timestamp field + a pre-check in `tracker.rs::tick()` ahead of the Q1 early-return. T4 adds a one-shot guarded reconcile in the first `RedrawRequested`, mirroring the existing `Resized` resize path.

**Tech Stack:** Rust, wgpu, winit 0.30, alacritty_terminal 0.24. `unsafe_code = "forbid"` (workspace). Cargo from `/home/bhengen/dev/vibeflow` (workspace root) — never `cd` into a crate.

**Spec:** `docs/superpowers/specs/2026-05-18-vibeflow-post-stage13-polish-design.md`

---

## Plan-level safety guards (re-state in EVERY implementer/reviewer dispatch)

1. **Do NOT delete or weaken any existing test.** A test that pinned a behavior a task *intentionally* changes is UPDATED (precondition swap / accurate assertion), never deleted/loosened — and called out explicitly in the report. Function-name diff before reporting DONE:
   `git show <base>:<file> | grep -E '^\s*(pub )?fn ' > /tmp/pre; git show HEAD:<file> | grep -E '^\s*(pub )?fn ' > /tmp/post; diff /tmp/pre /tmp/post` — explain every line.
2. **Reviewers are READ-ONLY.** Dispatch spec/quality reviewers with hard constraints: NO `git checkout/switch/branch/reset/stash/clean/commit/add/rm`, NO `rm`/`mv`; stay on the working branch; verify `git rev-parse --abbrev-ref HEAD` before+after. (Per `lesson_review_subagent_destructive`.)
3. **Controller runs `git status --short` after EVERY task** and confirms the branch/HEAD — per-task `--stat`/SHA gates cannot see unstaged working-tree drift (per `lesson_subagent_amend_drift`). Subagents doing `git commit --amend` MUST `git add` first and prove the change is in the commit via `git show HEAD --stat`.
4. **Four quality gates per task, all green before commit, from the workspace root:** `cargo fmt --all`; `cargo build --workspace`; `cargo test --workspace`; `cargo clippy --all-targets -- -D warnings`.
5. **A1/A2 invariant intact:** `grep -rn "colors_mut\|NamedColor::Bold\|NamedColor::CursorText" crates/vibeflow/src` returns only doc-comment mentions. No task writes color into `Term` or uses non-existent `NamedColor` variants.
6. **Known pre-existing flaky:** `tier_3_arms_on_rising_edge_even_without_real_output` can flake under full parallel; passes in isolation; unrelated. If it flakes, re-run isolated and report both — not a regression.

## Pre-execution senior review (workflow step, not a task)

Before T1 dispatch, run a Sonnet `general-purpose` review of THIS plan vs actual source. Reviewer prompt sketch:
> Read this plan end-to-end and the files it modifies, plus `alacritty_terminal-0.24.2/src/term/color.rs` (`Colors`, `Index<NamedColor>` → `Option<Rgb>`) and `vte-0.13.1/src/ansi.rs` (`NamedColor::Background` exists; `Rgb { r,g,b: u8 }`). Verify the code blocks compile against real APIs and the file:line anchors still match. Categorize Critical/Important/Minor/Verified-correct; apply Critical fixes before T1.

(Reviewer constrained read-only / no git-fs mutation per guard #2.)

---

## File structure

| File | Status | Task |
|---|---|---|
| `crates/vibeflow/src/render/mod.rs` | TOUCHED | T1 (clear color), T4 (no — T4 is window.rs) |
| `crates/vibeflow/src/config/schema.rs` | TOUCHED | T2 (`AiSection.explicit_stale_state_s`) |
| `crates/vibeflow/src/config/mod.rs` | TOUCHED | T2 (`Ai` field, default_values, apply_ai, tests) |
| `crates/vibeflow/src/session/tracker.rs` | TOUCHED | T2 (`TrackerConfig` field + Default), T3 (mechanism + tests) |
| `crates/vibeflow/src/window.rs` | TOUCHED | T2 (`apply_config` tracker_cfg), T4 (struct field + first-redraw reconcile) |

T1 depends on nothing. T3 depends on T2 (uses `TrackerConfig.explicit_stale_state`). T4 depends on nothing. Order: T1 → T2 → T3 → T4.

---

## Task 1: Theme-driven clear color

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs` (add a private helper; change the `LoadOp::Clear` arg ~line 560; `active_theme_colors` is computed at ~line 295 in the same `render()` fn so it is in scope at the pass)

Verified current state: `const CLEAR_COLOR: wgpu::Color = wgpu::Color { r:0x0e/255, g:0x0e/255, b:0x12/255, a:1.0 }` (~61). In `pub fn render(...)` (~252): `let active_theme_colors: Option<alacritty_terminal::term::color::Colors> = app.tabs().get(app.active()).and_then(|s| s.theme_colors);` (~295). Later in the SAME fn (~552-560): `encoder.begin_render_pass(... ops: wgpu::Operations { load: wgpu::LoadOp::Clear(CLEAR_COLOR), store: Store } ...)`.

- [ ] **Step 1: Write the failing test.** Append to `render/mod.rs`'s `#[cfg(test)] mod tests` (create the module if none — check with `grep -n "mod tests" crates/vibeflow/src/render/mod.rs`; if absent add `#[cfg(test)] mod tests { use super::*; ... }` at end of file):

```rust
    #[test]
    fn clear_color_uses_theme_bg_else_fallback() {
        use alacritty_terminal::term::color::Colors;
        use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
        // No theme → fallback constant.
        assert_eq!(theme_clear_color(None), CLEAR_COLOR);
        // Theme with a Background slot → that color, alpha 1.0.
        let mut c = Colors::default();
        c[NamedColor::Background] = Some(Rgb { r: 0x20, g: 0x40, b: 0x60 });
        let got = theme_clear_color(Some(&c));
        assert!((got.r - 0x20 as f64 / 255.0).abs() < 1e-9);
        assert!((got.g - 0x40 as f64 / 255.0).abs() < 1e-9);
        assert!((got.b - 0x60 as f64 / 255.0).abs() < 1e-9);
        assert_eq!(got.a, 1.0);
        // Theme present but Background unset → fallback.
        let empty = Colors::default();
        assert_eq!(theme_clear_color(Some(&empty)), CLEAR_COLOR);
    }
```

- [ ] **Step 2: Run it; expect failure (function not defined).**

`cargo test --package vibeflow --lib render::tests::clear_color_uses_theme_bg_else_fallback 2>&1 | tail -10`
Expected: compile error `cannot find function theme_clear_color`.

- [ ] **Step 3: Add the helper.** Place it near `CLEAR_COLOR` (just after the const, module level, private):

```rust
/// Stage 13 follow-up: the wgpu clear color for a frame. Uses the active
/// session's resolved theme background when set, so the uncovered
/// remainder strip (`width % cell_w` / `height % cell_h`, never covered by
/// a cell rect) matches the theme instead of the hardcoded default.
/// Falls back to `CLEAR_COLOR` when there is no theme / no Background slot.
fn theme_clear_color(
    theme_colors: Option<&alacritty_terminal::term::color::Colors>,
) -> wgpu::Color {
    use alacritty_terminal::vte::ansi::NamedColor;
    match theme_colors.and_then(|c| c[NamedColor::Background]) {
        Some(rgb) => wgpu::Color {
            r: rgb.r as f64 / 255.0,
            g: rgb.g as f64 / 255.0,
            b: rgb.b as f64 / 255.0,
            a: 1.0,
        },
        None => CLEAR_COLOR,
    }
}
```

- [ ] **Step 4: Run the test; expect PASS.**

`cargo test --package vibeflow --lib render::tests::clear_color_uses_theme_bg_else_fallback 2>&1 | tail -5`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Wire it into the render pass.** In `render()`, immediately before the `encoder.begin_render_pass(...)` block (the one labeled `"vibeflow-frame-pass"` with `LoadOp::Clear`), add:

```rust
        let frame_clear = theme_clear_color(active_theme_colors.as_ref());
```

Then change `load: wgpu::LoadOp::Clear(CLEAR_COLOR),` → `load: wgpu::LoadOp::Clear(frame_clear),`.

(Confirm `active_theme_colors` is still in scope at that point — it is a `let` bound earlier in the same `render()` fn at ~line 295 and not moved; `.as_ref()` borrows the `Option<Colors>`.)

- [ ] **Step 6: Quality gate (all four) + fn-name diff.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | grep "test result"
cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
git show HEAD:crates/vibeflow/src/render/mod.rs | grep -E '^\s*(pub )?fn ' > /tmp/pre_r 2>/dev/null || true
```
All green. Workspace lib test count = previous + 1 (the new test).

- [ ] **Step 7: Commit.**

```bash
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat: drive wgpu clear color from active theme background"
```

---

## Task 2: `[ai] explicit_stale_state_s` config plumbing (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs` (`AiSection`)
- Modify: `crates/vibeflow/src/config/mod.rs` (`Ai`, `default_values`, `apply_ai`, tests)
- Modify: `crates/vibeflow/src/session/tracker.rs` (`TrackerConfig` field + `Default`)
- Modify: `crates/vibeflow/src/window.rs` (`apply_config` `tracker_cfg` literal ~627)

Verified current state:
- schema.rs ~104 `pub struct AiSection { pub tools: Option<Vec<String>>, pub heuristic_silence_ms: Option<u64>, pub stale_state_timeout_s: Option<u64>, pub debounce_ms: Option<u64>, pub foreground_check_interval_ms: Option<u64> }` (`#[serde(default, deny_unknown_fields)]`-style like siblings — match the exact attrs on `AiSection`).
- mod.rs ~35 `pub struct Ai { pub tools: Vec<String>, pub heuristic_silence_ms: u64, pub stale_state_timeout_s: u64, pub debounce_ms: u64, pub foreground_check_interval_ms: u64 }`.
- mod.rs `default_values()` ~232 `ai: Ai { tools: vec![...], heuristic_silence_ms: 4000, stale_state_timeout_s: 30, debounce_ms: ..., foreground_check_interval_ms: ... }`.
- mod.rs `fn apply_ai(schema: schema::AiSection, resolved: &mut Ai)` ~574 — `if let Some(v) = schema.X { resolved.X = v; }` per field.
- tracker.rs ~42 `pub struct TrackerConfig { pub debounce: Duration, pub heuristic_silence: Duration, pub stale_state: Duration }`; `impl Default for TrackerConfig { fn default() -> Self { Self { debounce: from_millis(100), heuristic_silence: from_millis(4000), stale_state: from_secs(30) } } }`.
- window.rs `apply_config` ~627 `let tracker_cfg = crate::session::tracker::TrackerConfig { debounce: from_millis(ai.debounce_ms), heuristic_silence: from_millis(ai.heuristic_silence_ms), stale_state: from_secs(ai.stale_state_timeout_s) };`.

- [ ] **Step 1: Schema test (TDD).** Append to `config/schema.rs`'s `mod tests` (find the existing `[ai]` parse test, e.g. `ai_section_parses...`, and mirror its idiom for `super::ConfigFile`):

```rust
    #[test]
    fn ai_section_parses_explicit_stale_state_s() {
        let toml = r#"
[ai]
explicit_stale_state_s = 120
"#;
        let cs: super::ConfigFile = toml::from_str(toml).expect("parse");
        let a = cs.ai.expect("ai present");
        assert_eq!(a.explicit_stale_state_s, Some(120));
    }
```

- [ ] **Step 2: Run; expect `no field 'explicit_stale_state_s'`.**

`cargo test --package vibeflow --lib config::schema::tests::ai_section_parses_explicit_stale_state_s 2>&1 | tail -8`

- [ ] **Step 3: Add the schema field.** In `AiSection` (schema.rs ~104), after `foreground_check_interval_ms`:

```rust
    /// Stage 13 follow-up: for an explicit (OSC-1338) session, after this
    /// many seconds with no new frame the tab "de-escalates" — a stuck
    /// Working resets to Active and Tier-3 re-arms (see tracker). `0`
    /// disables (keeps Q1 "explicit = authoritative forever").
    pub explicit_stale_state_s: Option<u64>,
```

- [ ] **Step 4: mod.rs resolved field + default + apply + tests.**

In `Ai` (mod.rs ~35) after `foreground_check_interval_ms`:
```rust
    pub explicit_stale_state_s: u64,
```
In `default_values()`'s `ai: Ai { ... }` literal (after `foreground_check_interval_ms: <value>,`):
```rust
                explicit_stale_state_s: 300,
```
In `apply_ai` (after the `foreground_check_interval_ms` block):
```rust
    if let Some(v) = schema.explicit_stale_state_s {
        resolved.explicit_stale_state_s = v;
    }
```
Append to `config/mod.rs`'s `mod tests` (mirror the existing `ai_*` load tests' idiom — tempfile + `Config::load`):
```rust
    #[test]
    fn ai_explicit_stale_state_default_is_300() {
        let cf = Config::default_values();
        assert_eq!(cf.ai.explicit_stale_state_s, 300);
    }

    #[test]
    fn ai_explicit_stale_state_load_override_and_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ai]\nexplicit_stale_state_s = 0\n").expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.ai.explicit_stale_state_s, 0);
    }
```

- [ ] **Step 5: TrackerConfig field + Default.** tracker.rs ~42, add to `TrackerConfig` after `stale_state`:
```rust
    /// Stage 13 follow-up: for an explicit (OSC-1338) session, after this
    /// long with no new frame the fuse de-escalates the session (Working→
    /// Active, Tier-3 re-arms). `Duration::ZERO` disables (Q1 behavior).
    pub explicit_stale_state: Duration,
```
In `impl Default for TrackerConfig` add `explicit_stale_state: Duration::from_secs(300),`.

- [ ] **Step 6: Wire config → TrackerConfig in window.rs `apply_config` (~627).** Add to the `tracker_cfg` struct literal:
```rust
            explicit_stale_state: std::time::Duration::from_secs(ai.explicit_stale_state_s),
```
(`app.rs::default_tracker_config()` is `TrackerConfig::default()` and existing app.rs/test `TrackerConfig { .. , ..TrackerConfig::default() }` literals automatically pick up the new field via `Default` — no other call sites need editing. Confirm by build.)

- [ ] **Step 7: Run config + tracker tests.**

`cargo test --package vibeflow --lib config 2>&1 | grep "test result"`
`cargo test --package vibeflow --lib tracker 2>&1 | grep "test result"`
Expected: the 3 new config tests pass; tracker tests still pass (the new `TrackerConfig` field with a `Default` doesn't change behavior yet — T3 consumes it).

- [ ] **Step 8: Quality gate + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | grep "test result"
cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
git add crates/vibeflow/src/config/schema.rs crates/vibeflow/src/config/mod.rs crates/vibeflow/src/session/tracker.rs crates/vibeflow/src/window.rs
git commit -m "feat(config): [ai] explicit_stale_state_s (default 300s, 0=disabled)"
```

---

## Task 3: Tracker explicit-stale fuse (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs` (`AiStateTracker` struct + `new()` + `on_input` AiFrame arm + `tick()` + tests)

Verified current state (post-Q1): struct `AiStateTracker { state, config, last_event_at: Option<Instant>, last_output_at: Option<Instant>, heuristic_active: bool, explicit_seen: bool }`; `new()` inits all incl. `explicit_seen: false`; `on_input` `TrackerInput::AiFrame(frame) => { self.explicit_seen = true; self.transition_to(frame.state.into(), now) }`; `tick(&mut self, now)` begins with `if self.explicit_seen { return false; }` then heuristic-silence then stale-state. `transition_to` applies the 100 ms debounce.

- [ ] **Step 1: Write failing tests.** Append to tracker.rs `mod tests` (match the existing idiom: `AiStateTracker::new(TrackerConfig { explicit_stale_state: Duration::from_millis(500), ..TrackerConfig::default() })`, `now`, `on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now)`, `tick(now + Duration::from_millis(...))`; imports `State`, `Frame`, `TabState`, `TrackerInput`, `Duration`, `PromptMarker` already used in that module):

```rust
    #[test]
    fn explicit_stale_fuse_resets_working_to_active() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        assert_eq!(t.state(), TabState::Working);
        // Before the fuse: still authoritative, inert.
        assert!(!t.tick(now + Duration::from_millis(400)));
        assert_eq!(t.state(), TabState::Working);
        // After the fuse: Working → Active.
        assert!(t.tick(now + Duration::from_millis(600)));
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn explicit_stale_fuse_keeps_waiting_amber_but_dearms_explicit() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            stale_state: Duration::from_millis(50),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Waiting)), now);
        assert_eq!(t.state(), TabState::Waiting);
        // Fuse fires: Waiting stays (amber persists), returns false (no change).
        assert!(!t.tick(now + Duration::from_millis(600)));
        assert_eq!(t.state(), TabState::Waiting);
        // explicit_seen is now cleared: a Prompt(CommandStart) (Tier-3,
        // non-explicit) drives Working — proving the session de-escalated.
        assert!(t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandStart),
            now + Duration::from_millis(700),
        ));
        assert_eq!(t.state(), TabState::Working);
        // And the now-ungated stale-state did NOT silently reclaim the
        // earlier Waiting before that activity (last_event_at was nulled):
        // re-do the Waiting case and tick well past stale_state with no input.
        let mut t2 = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            stale_state: Duration::from_millis(50),
            ..TrackerConfig::default()
        });
        let n2 = Instant::now();
        t2.on_input(TrackerInput::AiFrame(Frame::new(State::Waiting)), n2);
        assert!(!t2.tick(n2 + Duration::from_millis(600))); // fuse fires
        assert!(!t2.tick(n2 + Duration::from_millis(5000))); // long after; no reclaim
        assert_eq!(t2.state(), TabState::Waiting, "amber must persist absent activity");
    }

    #[test]
    fn explicit_stale_fuse_not_premature_when_frames_keep_arriving() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        // A fresh frame at +400ms refreshes last_explicit_at.
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now + Duration::from_millis(400));
        // +600ms is >500ms from t0 but only 200ms from the last frame → no fuse.
        assert!(!t.tick(now + Duration::from_millis(600)));
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn explicit_stale_fuse_disabled_when_zero_keeps_q1_behavior() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::ZERO,
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        // Far past any threshold; with the fuse disabled, tick stays inert
        // (Q1: explicit = authoritative forever).
        assert!(!t.tick(now + Duration::from_secs(3600)));
        assert_eq!(t.state(), TabState::Working);
    }
```

- [ ] **Step 2: Run; expect failures** (e.g. `explicit_stale_fuse_resets_working_to_active` fails because `tick()` early-returns `false` on `explicit_seen` regardless of the new field — Working stays Working forever; also no `last_explicit_at`).

`cargo test --package vibeflow --lib tracker 2>&1 | grep -E "explicit_stale_fuse|test result"`

- [ ] **Step 3: Add `last_explicit_at` field + init.** In `AiStateTracker` struct, after `explicit_seen: bool,`:
```rust
    /// Stage 13 follow-up: `Instant` of the most recent explicit OSC 1338
    /// frame. Drives the explicit-stale fuse in `tick()`. `None` until the
    /// first frame; refreshed on every `AiFrame`.
    last_explicit_at: Option<Instant>,
```
In `new()` add `last_explicit_at: None,`.

- [ ] **Step 4: Set it in the AiFrame arm.** Change the `on_input` arm to:
```rust
            TrackerInput::AiFrame(frame) => {
                self.explicit_seen = true;
                self.last_explicit_at = Some(now);
                self.transition_to(frame.state.into(), now)
            }
```

- [ ] **Step 5: Replace the Q1 early-return in `tick()` with the fuse.** The current head of `tick()` is the comment block + `if self.explicit_seen { return false; }`. Replace JUST that `if self.explicit_seen { return false; }` (keep the Q1 comment block above it, and add the fuse rationale) with:

```rust
        // Stage 13 follow-up: explicit-stale fuse. While explicit (Tier-1
        // authoritative) we stay inert (Q1) UNLESS the fuse is enabled
        // (explicit_stale_state > 0) AND no frame has arrived for that long
        // — then the self-reporting tool is presumed gone: de-escalate.
        if self.explicit_seen {
            let stale = self.config.explicit_stale_state > Duration::ZERO
                && self
                    .last_explicit_at
                    .map(|l| now.saturating_duration_since(l) >= self.config.explicit_stale_state)
                    .unwrap_or(false);
            if !stale {
                return false; // still authoritative (Q1): inert
            }
            // Fuse fires: this session is no longer self-reporting.
            self.explicit_seen = false;
            self.last_explicit_at = None;
            match self.state {
                TabState::Working => {
                    // Dead hook mid-Working → recover to neutral.
                    self.state = TabState::Active;
                    self.last_event_at = Some(now);
                    return true;
                }
                TabState::Waiting => {
                    // Headline "needs you" cue persists. Null the stale-state
                    // baseline so the now-ungated stale timeout cannot silently
                    // reclaim it absent activity; the next prompt-marker /
                    // heuristic transition (Tier-3, now re-armed) moves it.
                    self.last_event_at = None;
                    return false;
                }
                _ => {
                    // Active/Idle/Done: just de-escalate; fall through so
                    // Tier-3 (below) governs normally from here.
                }
            }
        }
```

(Everything after this — the heuristic-silence and stale-state blocks — is UNCHANGED and now reached only when `!explicit_seen` (either never explicit, or just de-escalated via the `_ =>` arm). Verify the existing two blocks below remain byte-identical.)

- [ ] **Step 6: Run the new tests; expect PASS.**

`cargo test --package vibeflow --lib tracker 2>&1 | grep -E "explicit_stale_fuse|test result"`
Expected: 4 new tests pass.

- [ ] **Step 7: Verify existing Q1 / tracker tests still green.**

`cargo test --package vibeflow --lib tracker 2>&1 | grep "test result"`
All pass. The Q1 tests use `TrackerConfig::default()` → `explicit_stale_state = 300s`; they `tick()` at small offsets (ms / a few s) far below 300 s, so the fuse never fires and Q1 behavior is unchanged. **If any Q1 test ticks ≥300 s after an AiFrame and now changes outcome:** that test must get an intent-preserving precondition tweak — add `explicit_stale_state: Duration::ZERO` (or a large value) to its `TrackerConfig` so it still asserts the Q1-authoritative behavior it was written for. Report any such tweak explicitly (it is an allowed update, not a weakening — same assertion, fuse-disabled precondition). Do NOT delete/loosen.

- [ ] **Step 8: Quality gate + fn-name diff + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | grep "test result"
cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
# fn-name diff: only the 4 new test fns added; struct/new/on_input/tick keep names
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(tracker): explicit-stale fuse — Working→Active + Tier-3 re-arm"
```

---

## Task 4: Startup grid-size race — one-shot first-redraw reconcile

**Files:**
- Modify: `crates/vibeflow/src/window.rs` (`WindowApp` struct: add `initial_size_reconciled: bool`; `WindowApp::new` `Self { }` init; first `WindowEvent::RedrawRequested` handler ~995)

Verified current state: `WindowApp` struct ~200 (fields incl. `window: Option<Arc<Window>>`, `renderer: Option<Renderer>`, `app: App`, plus Stage-11/12/13 caches like `snap_on_esc`, `bell_mode`, `theme_registry`). `pixels_to_grid(width_px, height_px, cell_w, cell_h) -> (u16, u16)` at ~23. `WindowEvent::Resized(new_size)` handler ~1026: `renderer.resize(w,h)`; `bar_h = tab_bar_height_px(cell_h)`; `visible_h = new_size.height.saturating_sub(bar_h)`; `(rows, cols) = pixels_to_grid(new_size.width, visible_h, cell_w, cell_h)`; `self.app.resize_all(rows, cols)`. `WindowEvent::RedrawRequested` handler ~995 currently: get `term`, `renderer`, `match renderer.render(...)`. `resumed()` ~965 does the best-effort initial resize from `renderer.surface_size()` (KEEP AS-IS).

- [ ] **Step 1: Add the guard field.** In the `WindowApp` struct, near the other Stage-13 cache `bool`s (e.g. by `snap_on_esc`), add:
```rust
    /// Stage 13 follow-up: false until the first `RedrawRequested` reconciles
    /// the grid to the true `window.inner_size()`. `resumed()` sizes from
    /// `renderer.surface_size()`, which some compositors/VNC report as the
    /// requested size before the real window size is final; this one-shot
    /// reconcile corrects the grid before the first visible frame.
    initial_size_reconciled: bool,
```
In `WindowApp::new`'s `Self { ... }` literal add `initial_size_reconciled: false,`.

- [ ] **Step 2: Write a failing test for the reconcile decision.** `pixels_to_grid` is pure and already testable. Add to `window.rs` `mod tests` (find `mod tests` via grep; mirror the existing `pixels_to_grid` test idiom — e.g. `pixels_to_grid_uses_floor_division`):

```rust
    #[test]
    fn reconcile_recomputes_grid_for_true_window_size() {
        // resumed() may have sized for the requested 960x600; the real
        // window is larger. The reconcile must recompute via pixels_to_grid
        // on the true size (tab-bar strip reserved) and yield more rows/cols.
        let cell_w = 10u32;
        let cell_h = 20u32;
        let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
        let requested = super::pixels_to_grid(960, 600u32.saturating_sub(bar_h), cell_w, cell_h);
        let actual = super::pixels_to_grid(1920, 1080u32.saturating_sub(bar_h), cell_w, cell_h);
        assert!(actual.0 > requested.0, "more rows on the larger real window");
        assert!(actual.1 > requested.1, "more cols on the larger real window");
    }
```

- [ ] **Step 3: Run; expect PASS already** (this pins the math the reconcile relies on; `pixels_to_grid` + `tab_bar_height_px` already exist). If it does not compile, fix the `super::`/path prefix to match how sibling `pixels_to_grid` tests call it.

`cargo test --package vibeflow --lib window::tests::reconcile_recomputes_grid_for_true_window_size 2>&1 | tail -5`
Expected: PASS. (This is a guard test; the behavioral wiring in Step 4 is covered by the manual VNC smoke walk since it needs a real window/compositor.)

- [ ] **Step 4: Add the one-shot reconcile at the top of the `RedrawRequested` handler.** In `window_event`, the `WindowEvent::RedrawRequested => { ... }` arm: insert this BEFORE the existing `let term = self.app.active_term();` line:

```rust
                if !self.initial_size_reconciled {
                    self.initial_size_reconciled = true;
                    // Re-sync the grid to the TRUE window size now that the
                    // window is mapped (resumed() may have used the requested
                    // size). Same path as WindowEvent::Resized.
                    if let (Some(window), Some(renderer)) =
                        (self.window.as_ref(), self.renderer.as_ref())
                    {
                        let size = window.inner_size();
                        let (cell_w, cell_h) = renderer.cell_pitch();
                        let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
                        let visible_h = size.height.saturating_sub(bar_h);
                        let (rows, cols) =
                            pixels_to_grid(size.width, visible_h, cell_w, cell_h);
                        if let Err(e) = self.app.resize_all(rows, cols) {
                            tracing::warn!(error = %e, rows, cols, "initial reconcile resize failed");
                        }
                    }
                }
```

(Borrow note: this block ends — and drops the `window`/`renderer` shared borrows — before the existing `let Some(renderer) = self.renderer.as_mut()` in the same arm. If the borrow checker objects because `self.window`/`self.renderer` are read here and `self.renderer` is `as_mut()` just below, the read block is a separate statement scope and completes first; if a conflict still arises, hoist the computed `(rows, cols)` into locals inside the `if` and call `self.app.resize_all` there — which only borrows `self.app`, disjoint from `self.renderer`. The pattern mirrors the `Resized` handler which already does `cell_pitch` read then `resize` then `resize_all` without conflict.)

- [ ] **Step 5: Run window tests + workspace.**

```bash
cargo test --package vibeflow --lib window 2>&1 | grep "test result"
cargo build --workspace 2>&1 | tail -3
```

- [ ] **Step 6: Quality gate + fn-name diff + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | grep "test result"
cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
# fn-name diff window.rs: only + the new test fn; no production fn renamed/removed
git add crates/vibeflow/src/window.rs
git commit -m "fix(window): one-shot first-redraw grid-size reconcile"
```

---

## Manual smoke walk (after T4 passes, on slmbeast VNC)

```bash
cargo build --release
pkill -f 'target/release/vibeflow' 2>/dev/null
DISPLAY=:1 RUST_LOG=vibeflow=info,warn setsid /home/bhengen/dev/vibeflow/target/release/vibeflow >/tmp/vf.log 2>&1 & disown
```

1. **Theme bg covers the window:** import + `[colors] preset` a non-default theme; open vibeflow at a large/maximized VNC window. The terminal background fills the whole area from the FIRST frame (no large default-colored region; no thin mismatched bottom/right strip) — without needing a manual resize. (T1 + T4 together.)
2. **Resize still correct:** drag-resize the window; bg still fills; grid reflows. (Existing `Resized` path unaffected.)
3. **Explicit-stale fuse (Working):** in a tab run a quick OSC test or `claude`; force a stuck-Working (e.g. `vibeflow-emit working --tool=claude` written to the tab's tty, then no further frames). With `[ai] explicit_stale_state_s` set low (e.g. `5`) for the test, after ~5 s the stripe leaves blue → Active (neutral). Restore default after.
4. **Explicit-stale (Waiting persists):** emit `waiting`, then no frames for > the configured fuse; the amber "needs you" stripe MUST remain (does not silently flip to Active). Then a shell prompt / new command moves it (Tier-3 re-armed).
5. **`explicit_stale_state_s = 0` disables:** set `0`, emit `working`, wait well past any prior threshold — stays blue (Q1 authoritative-forever preserved).
6. **Hot-reload:** change `[ai] explicit_stale_state_s` in config, save; new value applies to existing tabs (via `apply_config`).

Fix anything surfaced; each fix its own conventional commit.

## Senior holistic review (after smoke walk)

Dispatch a Sonnet-tier holistic review (read-only / no git-fs mutation): does T1's `theme_clear_color` correctly fall back; does T3's fuse honor the exact spec semantics (Working→Active; Waiting persists with `last_event_at=None`; `0` disables; frames refresh the timer; de-escalated session correctly resumes Tier-3); is T4's reconcile genuinely one-shot and borrow-safe; do all `TrackerConfig` construction sites compile with the new field; A1/A2 intact; any cross-file drift. Apply Critical; apply Important unless costly; note Minor. Then `superpowers:finishing-a-development-branch` → merge to `main` `--no-ff` (no separate tag — v0.1/Stage-14 track).

## Plan self-review

Spec coverage: §1 → T1 ✅; §2 → T4 ✅ (one-shot first-redraw reconcile, `resumed()` kept); §3 → T2 (config knob) + T3 (mechanism incl. the Waiting `last_event_at=None` per the tightened spec) ✅; config/wiring/testing → T2 (schema/Ai/apply_ai/TrackerConfig/apply_config + hot-reload via existing plumbing) ✅. No spec requirement without a task.

Placeholder scan: no TBD/TODO; every code step shows complete code; tests are concrete with real assertions; no "similar to Task N".

Type consistency: `theme_clear_color(Option<&Colors>) -> wgpu::Color` (T1, used T1 step5). `AiSection.explicit_stale_state_s: Option<u64>` → `Ai.explicit_stale_state_s: u64` → `TrackerConfig.explicit_stale_state: Duration` (T2, consumed T3 `tick()` + window.rs `apply_config`). `AiStateTracker.last_explicit_at: Option<Instant>` (T3 struct/new/on_input/tick consistent). `initial_size_reconciled: bool` (T4 struct/new/handler consistent). `pixels_to_grid`/`tab_bar_height_px`/`resize_all`/`cell_pitch` reused with existing signatures.
