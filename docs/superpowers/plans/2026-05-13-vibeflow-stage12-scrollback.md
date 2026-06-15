# vibeflow Stage 12 — Scrollback rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make scrollback history visible: renderer walks rows with `term.grid().display_offset()`, fade-in scrollbar on user scroll, mouse-wheel + touchpad + keyboard navigation, snap-to-bottom on keystroke. Mouse-wheel-in-mouse-mode passes through to TUI apps via extended `mouse_encoder`.

**Architecture:** Leverage `alacritty_terminal::Term::scroll_display(Scroll::*)` and `grid.display_offset()` directly — no parallel scrollback buffer. New `render::scrollbar::ScrollbarFade` (per-session fade-state) + `build_scrollbar_rects` (pure function emitting track+thumb rects). New `[scrollback]` config section with three knobs. `mouse_encoder::Button` enum gains `WheelUp` (xterm code 4) and `WheelDown` (xterm code 5) variants.

**Tech Stack:** Rust, winit 0.30 (`WindowEvent::MouseWheel`, `MouseScrollDelta`), wgpu, alacritty_terminal 0.24 (`Scroll`, `Term::scroll_display`, `Dimensions::display_offset`). No new external crates.

**Spec:** `docs/superpowers/specs/2026-05-13-vibeflow-stage12-scrollback-design.md`

---

## Critical Stage 12 safety guards (re-state in every implementer dispatch prompt)

Per the `feedback_implementer_safety` lesson, every dispatch must restate:

1. **DO NOT delete or weaken any existing test in any file you touch.** Function-name diff before reporting DONE:
   ```
   git show HEAD~1:<file> | grep -E '^\s*fn ' > /tmp/pre.txt
   git show HEAD:<file>   | grep -E '^\s*fn ' > /tmp/post.txt
   diff /tmp/pre.txt /tmp/post.txt
   ```
2. **Report deviations honestly.** Even tiny ones — renamed variables, swapped escape sequences, removed `use` lines.
3. **Cargo runs from `/path/to/vibeflow`.** Do not `cd` into crate dirs.
4. **Quality gate per task:** `cargo fmt --all`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`. All four must pass before commit.

## Pre-execution senior review (workflow step, not a task)

Before dispatching the first implementer for Task 1, run a Sonnet-tier `general-purpose` review per the `feedback_senior_review_plans` lesson. Reviewer prompt sketch:

> Read `docs/superpowers/plans/2026-05-13-vibeflow-stage12-scrollback.md`. Read the actual source it modifies — `crates/vibeflow/src/{render/{scrollbar.rs (will not yet exist), mod.rs, selection.rs, mouse_encoder.rs, tabs.rs}, session/session.rs, app.rs, window.rs, config/{schema.rs, mod.rs}}` — plus the alacritty_terminal 0.24 crate at `~/.cargo/registry/src/index.crates.io-*/alacritty_terminal-0.24.2/src/{term/mod.rs,grid/mod.rs}` (specifically `Scroll` enum at grid/mod.rs:73, `Term::scroll_display` at term/mod.rs:389, `TermMode` flags). Verify every API claim, type signature, modifier name, struct field, accessor existence in the plan. Categorize as Critical / Important / Minor / Verified-correct. Apply Critical fixes immediately; Important unless cost is high; Minor noted.

Apply the review's fixes inline before T1 dispatch.

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `crates/vibeflow/src/render/scrollbar.rs` | NEW | `ScrollbarFade` state machine; `build_scrollbar_rects` pure function; `ScrollbarColors` struct. Linux-and-everywhere; pure logic. |
| `crates/vibeflow/src/render/mod.rs` | TOUCHED | `Renderer.scrollbar_colors: ScrollbarColors` field + `set_scrollbar_colors` setter; `render()` appends scrollbar rects after bell-flash before context-menu; schedules redraw while fade > 0; cell-render pass reads `term.grid().display_offset()` (existing Term iteration already handles it). Submodule declaration. |
| `crates/vibeflow/src/render/selection.rs` | TOUCHED | `build_selection_rects` accepts `display_offset: usize`; translates negative-line indices to on-screen y when visible. |
| `crates/vibeflow/src/render/mouse_encoder.rs` | TOUCHED | `Button` enum gains `WheelUp` (xterm code 4) and `WheelDown` (xterm code 5). `encode_press` match updated. |
| `crates/vibeflow/src/session/session.rs` | TOUCHED | `PtySession.scrollbar_fade: ScrollbarFade` field. New methods: `scroll_by`, `scroll_to_top`, `scroll_to_bottom`, `display_offset`. `spawn()` initializes; `restart()` resets. |
| `crates/vibeflow/src/app.rs` | TOUCHED | `App.default_scrollbar_fade_ms: u64` field + `set_default_scrollbar_fade_ms` setter mirroring Stage 9/11 pattern. `new_tab` and `restart_active` propagate. |
| `crates/vibeflow/src/config/schema.rs` | TOUCHED | New `ScrollbackSection` schema struct; `ConfigFile.scrollback: Option<ScrollbackSection>` field. Two new `[colors]` schema keys (`scrollbar_track`, `scrollbar_thumb`). |
| `crates/vibeflow/src/config/mod.rs` | TOUCHED | New resolved `Scrollback` struct; `Config.scrollback: Scrollback` field; defaults in `default_values()`; `apply_scrollback` helper; two new resolved color fields. |
| `crates/vibeflow/src/window.rs` | TOUCHED | New `WindowEvent::MouseWheel` arm with mouse-mode gating. Chord handlers for Shift+PageUp/Down + Ctrl+Home/End. Snap-to-bottom on `key_to_bytes`-producing input. `apply_config` wires `[scrollback]` to App + per-session. `WindowApp.last_grid_size_lines: usize` cache + `wheel_lines_per_detent: u32` cache. |
| `crates/vibeflow/tests/scrollback.rs` | NEW | Integration tests against a real PTY. |

---

### Task 1: `render::scrollbar` module (TDD)

**Files:**
- Create: `crates/vibeflow/src/render/scrollbar.rs`
- Modify: `crates/vibeflow/src/render/mod.rs` (add `pub(crate) mod scrollbar;`)

- [ ] **Step 1: Create the module file.**

Write `/path/to/vibeflow/crates/vibeflow/src/render/scrollbar.rs`:

```rust
//! Stage 12: scrollbar fade-state + rect-build for the right-edge thumb.
//! Pure logic; no wgpu, no winit.

#![allow(dead_code)] // Renderer integration lands in Task 8; clean up there.

use std::time::Instant;

/// Color cache for the scrollbar track + thumb. Wired through Stage 9's
/// `[colors]` schema in Task 4; defaults to very faint white that's visible
/// on the dark Stage 9 background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarColors {
    pub track: [f32; 4],
    pub thumb: [f32; 4],
}

impl Default for ScrollbarColors {
    fn default() -> Self {
        Self {
            track: [1.0, 1.0, 1.0, 0.04],
            thumb: [1.0, 1.0, 1.0, 0.22],
        }
    }
}

/// Per-session fade-state for the scrollbar thumb. Stays at α=0 when the
/// session hasn't been scrolled recently; pops to α=1 the instant a scroll
/// happens; fades linearly back to 0 over `fade_ms` after the last activity.
#[derive(Debug, Clone)]
pub struct ScrollbarFade {
    last_scroll_at: Option<Instant>,
    fade_ms: u64,
}

impl ScrollbarFade {
    pub fn new(fade_ms: u64) -> Self {
        Self {
            last_scroll_at: None,
            fade_ms,
        }
    }

    pub fn mark_scrolled(&mut self, now: Instant) {
        self.last_scroll_at = Some(now);
    }

    /// 1.0 at the instant of activity, linearly decreasing to 0.0 at fade_ms.
    /// Returns 0.0 if never scrolled or fade has elapsed.
    pub fn alpha(&self, now: Instant) -> f32 {
        let Some(last) = self.last_scroll_at else {
            return 0.0;
        };
        let elapsed_ms = now.saturating_duration_since(last).as_millis() as u64;
        if elapsed_ms >= self.fade_ms {
            0.0
        } else {
            1.0 - (elapsed_ms as f32 / self.fade_ms as f32)
        }
    }

    /// Update the fade duration at runtime. Existing in-progress fade keeps
    /// its baseline (last_scroll_at) and is re-evaluated on next alpha() call.
    pub fn set_fade_ms(&mut self, fade_ms: u64) {
        self.fade_ms = fade_ms;
    }
}

const TRACK_WIDTH_PX: f32 = 8.0;
const THUMB_MIN_HEIGHT_PX: f32 = 20.0;
const TRACK_INSET_PX: f32 = 1.0;

/// Build the rect instances for the scrollbar at the current state. Returns
/// empty when there's nothing to draw (fade at 0 OR no scrollback content).
///
/// `display_offset` is how many lines into history the viewport is scrolled.
/// `history_size` is total history rows available.
/// `screen_lines` is the visible viewport height in rows.
/// `surface_size` is the wgpu surface size in physical pixels.
/// `bar_height_px` is the tab strip height (scrollbar starts below it).
pub fn build_scrollbar_rects(
    fade_alpha: f32,
    display_offset: usize,
    history_size: usize,
    screen_lines: usize,
    surface_size: (f32, f32),
    bar_height_px: f32,
    colors: ScrollbarColors,
) -> Vec<crate::render::tabs::RectInstance> {
    if fade_alpha <= 0.0 {
        return Vec::new();
    }
    if display_offset == 0 && history_size == 0 {
        return Vec::new();
    }

    let (surface_w, surface_h) = surface_size;
    let track_x = surface_w - TRACK_WIDTH_PX;
    let track_y = bar_height_px;
    let track_h = (surface_h - bar_height_px).max(0.0);
    if track_h < THUMB_MIN_HEIGHT_PX {
        return Vec::new();
    }

    let total_lines = (history_size + screen_lines).max(1) as f32;
    let visible_lines = screen_lines as f32;
    let thumb_h = (visible_lines / total_lines * track_h)
        .max(THUMB_MIN_HEIGHT_PX)
        .min(track_h);

    let max_thumb_y_offset = track_h - thumb_h;
    let history_for_fraction = history_size.max(1) as f32;
    let scroll_fraction = (display_offset as f32 / history_for_fraction).min(1.0);
    // display_offset == 0 -> thumb at bottom of track.
    // display_offset == history_size -> thumb at top.
    let thumb_y = track_y + max_thumb_y_offset * (1.0 - scroll_fraction);

    let track_color = scale_alpha(colors.track, fade_alpha);
    let thumb_color = scale_alpha(colors.thumb, fade_alpha);

    vec![
        crate::render::tabs::RectInstance::new(
            track_x, track_y, TRACK_WIDTH_PX, track_h, track_color,
        ),
        crate::render::tabs::RectInstance::new(
            track_x + TRACK_INSET_PX,
            thumb_y,
            TRACK_WIDTH_PX - 2.0 * TRACK_INSET_PX,
            thumb_h,
            thumb_color,
        ),
    ]
}

fn scale_alpha(mut c: [f32; 4], factor: f32) -> [f32; 4] {
    c[3] *= factor;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fade_returns_zero_when_never_scrolled() {
        let f = ScrollbarFade::new(1500);
        assert_eq!(f.alpha(Instant::now()), 0.0);
    }

    #[test]
    fn fade_returns_one_at_scroll_instant() {
        let mut f = ScrollbarFade::new(1500);
        let now = Instant::now();
        f.mark_scrolled(now);
        assert_eq!(f.alpha(now), 1.0);
    }

    #[test]
    fn fade_decreases_linearly_to_zero() {
        let mut f = ScrollbarFade::new(1000);
        let now = Instant::now();
        f.mark_scrolled(now);
        let half = f.alpha(now + Duration::from_millis(500));
        assert!((half - 0.5).abs() < 0.05, "half-elapsed alpha = {half}");
        let done = f.alpha(now + Duration::from_millis(1100));
        assert_eq!(done, 0.0);
    }

    #[test]
    fn set_fade_ms_updates_threshold() {
        let mut f = ScrollbarFade::new(1000);
        let now = Instant::now();
        f.mark_scrolled(now);
        f.set_fade_ms(500);
        // Past the new threshold but under the old.
        assert_eq!(f.alpha(now + Duration::from_millis(600)), 0.0);
    }

    #[test]
    fn build_scrollbar_rects_empty_when_alpha_zero() {
        let rects = build_scrollbar_rects(
            0.0,
            100,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert!(rects.is_empty());
    }

    #[test]
    fn build_scrollbar_rects_empty_when_at_bottom_no_history() {
        let rects = build_scrollbar_rects(
            1.0,
            0,
            0,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert!(rects.is_empty());
    }

    #[test]
    fn build_scrollbar_rects_returns_two_rects_for_normal_state() {
        let rects = build_scrollbar_rects(
            1.0,
            100,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn build_scrollbar_rects_thumb_at_bottom_when_display_offset_zero() {
        let rects = build_scrollbar_rects(
            1.0,
            0,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert_eq!(rects.len(), 2);
        // RectInstance fields are (x, y, w, h, color). Track is rects[0]; thumb is rects[1].
        let track_y = rects[0].pos_size[1];
        let track_h = rects[0].pos_size[3];
        let thumb_y = rects[1].pos_size[1];
        let thumb_h = rects[1].pos_size[3];
        let track_bottom = track_y + track_h;
        let thumb_bottom = thumb_y + thumb_h;
        assert!(
            (thumb_bottom - track_bottom).abs() < 1.0,
            "thumb should sit at bottom of track when display_offset=0; track_bottom={track_bottom}, thumb_bottom={thumb_bottom}"
        );
    }

    #[test]
    fn build_scrollbar_rects_thumb_min_height_clamps() {
        let rects = build_scrollbar_rects(
            1.0,
            5000,
            10000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        let thumb_h = rects[1].pos_size[3];
        assert!(
            thumb_h >= THUMB_MIN_HEIGHT_PX,
            "thumb should be >= MIN_HEIGHT_PX (20); got {thumb_h}"
        );
    }

    #[test]
    fn build_scrollbar_rects_alpha_scales_track_and_thumb() {
        let rects = build_scrollbar_rects(
            0.5,
            100,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        let track_alpha = rects[0].color[3];
        let thumb_alpha = rects[1].color[3];
        // Defaults: track=0.04, thumb=0.22. At fade=0.5: track=0.02, thumb=0.11.
        assert!((track_alpha - 0.02).abs() < 0.001, "track_alpha={track_alpha}");
        assert!((thumb_alpha - 0.11).abs() < 0.001, "thumb_alpha={thumb_alpha}");
    }
}
```

**Verified by senior pre-execution review:** `RectInstance` at `crates/vibeflow/src/render/tabs.rs:349` exposes `pub pos_size: [f32; 4]` (layout `[x, y, w, h]`) and `pub color: [f32; 4]`. Constructor `RectInstance::new(x, y, w, h, color)` is stable. `RectInstance` derives `Copy + Clone`.

- [ ] **Step 2: Add the module declaration to `render/mod.rs`.**

Find the existing `pub mod` block (near the top). Insert in alphabetical order — between `quad` and `selection`:

```rust
pub(crate) mod scrollbar;
```

`pub(crate)` because it's a render-internal helper exposed only to `Renderer::render`.

- [ ] **Step 3: Run the tests.**

```bash
cd /path/to/vibeflow
cargo test --package vibeflow --lib render::scrollbar::tests 2>&1 | tail -15
```
Expected: 9 passed (4 fade tests + 5 rects tests).

- [ ] **Step 4: Quality gate.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: all green.

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/render/scrollbar.rs crates/vibeflow/src/render/mod.rs
git commit -m "feat(stage12): scrollbar fade-state + build_scrollbar_rects (pure logic + 9 tests)"
```

---

### Task 2: `[scrollback]` config schema fields (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs`

- [ ] **Step 1: Add tests to `config::schema::tests`.**

Append:

```rust
    #[test]
    fn scrollback_section_parses_all_fields() {
        let toml = r#"
[scrollback]
history_lines = 5000
wheel_lines_per_detent = 5
scrollbar_fade_ms = 2000
"#;
        let cs: super::ConfigFile = toml::from_str(toml).expect("parse");
        let sb = cs.scrollback.expect("scrollback section present");
        assert_eq!(sb.history_lines, Some(5000));
        assert_eq!(sb.wheel_lines_per_detent, Some(5));
        assert_eq!(sb.scrollbar_fade_ms, Some(2000));
    }

    #[test]
    fn scrollback_section_missing_keeps_none() {
        let toml = "";
        let cs: super::ConfigFile = toml::from_str(toml).expect("parse");
        assert!(cs.scrollback.is_none());
    }

    #[test]
    fn scrollback_section_rejects_unknown_field() {
        let toml = r#"
[scrollback]
bogus_key = 1
"#;
        let r: Result<super::ConfigFile, _> = toml::from_str(toml);
        assert!(r.is_err());
    }
```

- [ ] **Step 2: Run; expect compile errors.**

```bash
cargo test --package vibeflow --lib config::schema::tests::scrollback 2>&1 | tail -10
```
Expected: build error — `cannot find type 'ScrollbackSection'` or `field 'scrollback' does not exist`.

- [ ] **Step 3: Add the schema struct.**

Add next to other `*Section` structs:

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrollbackSection {
    pub history_lines: Option<u32>,
    pub wheel_lines_per_detent: Option<u32>,
    pub scrollbar_fade_ms: Option<u64>,
}
```

Add the field to `ConfigFile`:

```rust
    pub scrollback: Option<ScrollbackSection>,
```

- [ ] **Step 4: Run tests.**

```bash
cargo test --package vibeflow --lib config::schema::tests::scrollback 2>&1 | tail -10
```
Expected: 3 passed.

- [ ] **Step 5: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/config/schema.rs
git commit -m "feat(stage12): [scrollback] config schema (deny_unknown_fields)"
```

---

### Task 3: `[scrollback]` resolved struct + defaults + apply (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/mod.rs`

Same pattern as Stage 11 T4 for `[ai]`.

- [ ] **Step 1: Add tests.**

Append to `config::tests`:

```rust
    #[test]
    fn scrollback_defaults_match_spec() {
        let cf = Config::default_values();
        assert_eq!(cf.scrollback.history_lines, 10000);
        assert_eq!(cf.scrollback.wheel_lines_per_detent, 3);
        assert_eq!(cf.scrollback.scrollbar_fade_ms, 1500);
    }

    #[test]
    fn scrollback_load_overrides_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[scrollback]
history_lines = 500
wheel_lines_per_detent = 1
"#,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.scrollback.history_lines, 500);
        assert_eq!(cf.scrollback.wheel_lines_per_detent, 1);
        // Unspecified keys keep defaults.
        assert_eq!(cf.scrollback.scrollbar_fade_ms, 1500);
    }

    #[test]
    fn scrollback_history_lines_zero_clamps_to_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
[scrollback]
history_lines = 0
"#).expect("write");
        let (cf, _) = Config::load(&path);
        assert_eq!(cf.scrollback.history_lines, 1, "0 should clamp to 1 per spec edge case");
    }
```

- [ ] **Step 2: Run; expect compile errors.**

```bash
cargo test --package vibeflow --lib config::tests::scrollback 2>&1 | tail -15
```

- [ ] **Step 3: Add the resolved struct.**

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Scrollback {
    pub history_lines: u32,
    pub wheel_lines_per_detent: u32,
    pub scrollbar_fade_ms: u64,
}
```

- [ ] **Step 4: Add field to `Config` and populate in `default_values()`.**

In `Config`:
```rust
    pub scrollback: Scrollback,
```

In `Config::default_values()` returned literal:
```rust
            scrollback: Scrollback {
                history_lines: 10000,
                wheel_lines_per_detent: 3,
                scrollbar_fade_ms: 1500,
            },
```

- [ ] **Step 5: Add `apply_scrollback` and wire into `Config::load`.**

Add the helper next to `apply_ai`:

```rust
fn apply_scrollback(schema: schema::ScrollbackSection, resolved: &mut Scrollback) {
    if let Some(v) = schema.history_lines {
        // Edge case: 0 would panic alacritty_terminal's grid construction. Clamp to 1.
        resolved.history_lines = v.max(1);
    }
    if let Some(v) = schema.wheel_lines_per_detent {
        resolved.wheel_lines_per_detent = v.max(1);
    }
    if let Some(v) = schema.scrollbar_fade_ms {
        resolved.scrollbar_fade_ms = v;
    }
}
```

In `Config::load` body (same place `apply_ai` is called):
```rust
        if let Some(s) = file.scrollback {
            apply_scrollback(s, &mut defaults.scrollback);
        }
```

- [ ] **Step 6: Run tests + workspace.**

```bash
cargo test --package vibeflow --lib config 2>&1 | tail -10
cargo test --workspace 2>&1 | tail -5
```
Expected: all pass including 3 new ones.

- [ ] **Step 7: Quality gate + commit.**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/config/mod.rs
git commit -m "feat(stage12): [scrollback] resolved struct + defaults + apply (clamps history_lines >= 1)"
```

---

### Task 4: `[colors] scrollbar_track` + `scrollbar_thumb` + Renderer setter

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs` (add two more keys to `ColorsSection`)
- Modify: `crates/vibeflow/src/config/mod.rs` (add two fields to `Colors`, populate defaults, extend apply)
- Modify: `crates/vibeflow/src/render/mod.rs` (add `scrollbar_colors` field + setter)

- [ ] **Step 1: Read existing `[colors]` pattern.**

Stage 9 added many menu_* keys with the same pattern. `grep -n 'menu_bg\|menu_border' crates/vibeflow/src/config/{schema.rs,mod.rs}` for reference. Stage 11's `apply_colors` parses hex strings via the existing `rgba()` helper.

- [ ] **Step 2: Add schema fields.**

In `ColorsSection`:
```rust
    pub scrollbar_track: Option<String>,
    pub scrollbar_thumb: Option<String>,
```

- [ ] **Step 3: Add resolved fields + defaults + apply.**

In `Colors`:
```rust
    pub scrollbar_track: [f32; 4],
    pub scrollbar_thumb: [f32; 4],
```

In `Config::default_values()` → `colors:` literal:
```rust
                scrollbar_track: [1.0, 1.0, 1.0, 0.04],
                scrollbar_thumb: [1.0, 1.0, 1.0, 0.22],
```

In `apply_colors` (find the existing function — Stage 9 + Stage 11 added many entries to it):
```rust
    apply(&schema.scrollbar_track, &mut resolved.scrollbar_track, errors);
    apply(&schema.scrollbar_thumb, &mut resolved.scrollbar_thumb, errors);
```

(Match the existing `apply` helper signature — read the file before editing.)

- [ ] **Step 4: Add a test for the defaults.**

In `config::tests`:
```rust
    #[test]
    fn scrollbar_colors_default_to_subtle_white() {
        let cf = Config::default_values();
        assert_eq!(cf.colors.scrollbar_track, [1.0, 1.0, 1.0, 0.04]);
        assert_eq!(cf.colors.scrollbar_thumb, [1.0, 1.0, 1.0, 0.22]);
    }
```

- [ ] **Step 5: Add Renderer field + setter.**

In `Renderer` struct (find `menu_colors` from Stage 10 T9 — mirror that pattern):

```rust
    /// Stage 12: scrollbar track + thumb colors. Defaults from
    /// `Config::default_values()` at construction; overwritten by
    /// `WindowApp::apply_config` from `[colors] scrollbar_*` keys.
    scrollbar_colors: crate::render::scrollbar::ScrollbarColors,
```

In `Renderer::new` (find the initialization of `menu_colors` for reference):
```rust
            scrollbar_colors: crate::render::scrollbar::ScrollbarColors::default(),
```

Add the setter in `impl Renderer` (next to `set_menu_colors`):
```rust
    pub fn set_scrollbar_colors(&mut self, colors: crate::render::scrollbar::ScrollbarColors) {
        self.scrollbar_colors = colors;
    }
```

- [ ] **Step 6: Run tests + workspace.**

```bash
cargo test --package vibeflow --lib 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: all green.

- [ ] **Step 7: Commit.**

```bash
git add crates/vibeflow/src/config/{schema.rs,mod.rs} crates/vibeflow/src/render/mod.rs
git commit -m "feat(stage12): [colors] scrollbar_track/thumb + Renderer setter"
```

---

### Task 5: `PtySession` Stage 12 fields + scroll methods + history_lines wiring (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`

- [ ] **Step 1: Add tests.**

Append to `session::session::tests`:

```rust
    #[test]
    fn scroll_by_zero_is_noop_no_fade() {
        let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
        let now = std::time::Instant::now();
        // Pre-fade state.
        assert_eq!(s.scrollbar_fade.alpha(now), 0.0);
        s.scroll_by(0, now);
        // No fade triggered.
        assert_eq!(
            s.scrollbar_fade.alpha(now),
            0.0,
            "scroll_by(0) should not arm the fade timer"
        );
    }

    #[test]
    fn scroll_by_nonzero_arms_fade() {
        let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
        let now = std::time::Instant::now();
        s.scroll_by(-5, now);
        assert!(s.scrollbar_fade.alpha(now) > 0.0, "fade should arm on nonzero scroll");
    }

    #[test]
    fn scroll_to_top_then_bottom_round_trips_display_offset() {
        let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
        let now = std::time::Instant::now();
        // Without history content the round-trip is trivial — both yield 0.
        // The test still validates the API does not panic.
        s.scroll_to_top(now);
        s.scroll_to_bottom(now);
        assert_eq!(s.display_offset(), 0);
    }

    #[test]
    fn display_offset_starts_at_zero() {
        let s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
        assert_eq!(s.display_offset(), 0);
    }
```

- [ ] **Step 2: Run; expect compile errors.**

```bash
cargo test --package vibeflow --lib session::session::tests::scroll 2>&1 | tail -10
```
Expected: build error — `no method named 'scroll_by'`, etc.

- [ ] **Step 3: Add the field to `PtySession` struct.**

```rust
    /// Stage 12: per-session scrollbar fade-state. Marked on user scroll;
    /// fades back to invisible after `scrollbar_fade_ms`.
    pub(crate) scrollbar_fade: crate::render::scrollbar::ScrollbarFade,
```

**Also extend `PtySession::spawn` signature to accept history_lines.** Senior pre-execution review caught this: without it, `[scrollback] history_lines` is silently ignored because `Term::new(TermConfig::default(), ...)` always uses the alacritty default (10000) regardless of user config.

Current signature (`session.rs:131`):
```rust
pub fn spawn(argv: &[&str], config: TrackerConfig) -> std::io::Result<Self>
```

New signature:
```rust
pub fn spawn(argv: &[&str], config: TrackerConfig, history_lines: usize) -> std::io::Result<Self>
```

Inside `spawn`, when constructing the `TermConfig`, set scrolling_history:
```rust
let term_config = alacritty_terminal::term::Config {
    scrolling_history: history_lines.max(1),
    ..Default::default()
};
let term = alacritty_terminal::term::Term::new(term_config, &term_size, alacritty_terminal::event::VoidListener);
```

Read the existing `Term::new` call at `session.rs:173` first to confirm the exact path and field name. The crate's `Config` is at `alacritty_terminal::term::Config` per the import in session.rs.

Initialize the new fields in `Ok(Self { ... })` after the Stage 11 fields:

```rust
            scrollbar_fade: crate::render::scrollbar::ScrollbarFade::new(1500),
```

1500 ms is the same default `Config::default_values()` uses. `App::new_tab` will overwrite both `scrollbar_fade.fade_ms` and the scrolling_history (via the new spawn param) from current config.

**All existing `PtySession::spawn(argv, tracker_config)` call sites need to be updated to pass a third argument.** Find via:
```bash
grep -n "PtySession::spawn" crates/vibeflow/src/ crates/vibeflow/tests/ 2>&1
```

Most likely call sites: `App::new_tab` (T6 covers this), `PtySession::restart` (also update — same default), and any existing test that spawns directly. For tests, pass `10000` as the third argument. For production code, pass `self.default_history_lines` (App field added in T6) or `Config::default_values().scrollback.history_lines`.

- [ ] **Step 4: Add the methods.**

In `impl PtySession`, next to `set_tracker_config`:

```rust
    /// Stage 12: scroll the display by `lines`. Negative = into history; positive = toward live.
    /// No-op when `lines == 0`. Marks the scrollbar fade timer when non-zero.
    pub fn scroll_by(&mut self, lines: i32, now: std::time::Instant) {
        if lines == 0 {
            return;
        }
        use alacritty_terminal::grid::Scroll;
        // alacritty Scroll::Delta convention: positive scrolls FORWARD (toward live).
        // Our public convention: negative `lines` = into history; positive = toward live.
        // Verify this sign during smoke walk; if reversed, swap the negation.
        self.term.scroll_display(Scroll::Delta(-lines));
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Stage 12: jump to top of history.
    pub fn scroll_to_top(&mut self, now: std::time::Instant) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Top);
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Stage 12: jump back to live viewport.
    pub fn scroll_to_bottom(&mut self, now: std::time::Instant) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Bottom);
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Stage 12: current display_offset (0 = at live viewport).
    pub fn display_offset(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        self.term.grid().display_offset()
    }

    /// Stage 12: read-only accessor for the scrollbar fade alpha at `now`.
    /// `pub fn` (not `pub(crate)`) so integration tests at
    /// `crates/vibeflow/tests/` can reach it across the compilation-unit
    /// boundary. Same lesson Stage 11 learned for `last_proc_check`.
    pub fn scrollbar_fade_alpha(&self, now: std::time::Instant) -> f32 {
        self.scrollbar_fade.alpha(now)
    }
```

- [ ] **Step 5: Run tests.**

```bash
cargo test --package vibeflow --lib session::session::tests::scroll 2>&1 | tail -10
cargo test --package vibeflow --lib session::session::tests::display_offset 2>&1 | tail -5
```
Expected: 4 passed.

- [ ] **Step 6: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/session/session.rs
git commit -m "feat(stage12): PtySession scroll_by/scroll_to_top/scroll_to_bottom + scrollbar_fade field"
```

---

### Task 6: `App` default-setter + new_tab propagation + restart_active propagation

**Files:**
- Modify: `crates/vibeflow/src/app.rs`

- [ ] **Step 1: Add tests.**

Append to `app::tests`:

```rust
    #[test]
    fn new_tab_inherits_default_scrollbar_fade_ms() {
        let mut app = App::new();
        app.set_default_scrollbar_fade_ms(2222);
        let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        let now = std::time::Instant::now();
        let s = &mut app.tabs_mut()[0];
        s.scrollbar_fade.mark_scrolled(now);
        // 2300ms past should be elapsed past 2222ms threshold → 0.0.
        assert_eq!(s.scrollbar_fade.alpha(now + std::time::Duration::from_millis(2300)), 0.0);
        // Just under threshold should still be > 0 (linear fade — small positive value).
        let near_end = s.scrollbar_fade.alpha(now + std::time::Duration::from_millis(2100));
        assert!(near_end > 0.0 && near_end < 0.1, "near-threshold fade alpha out of range: {near_end}");
    }

    #[test]
    fn restart_active_propagates_scrollbar_fade() {
        let mut app = App::new();
        app.set_default_scrollbar_fade_ms(777);
        // Spawn a short-lived child so it can be restarted.
        let _ = app.new_tab(&["/bin/sh", "-c", "true"]).expect("spawn");
        // Wait for the child to die.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline && app.tabs()[0].is_alive() {
            let _ = app.poll_all(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.restart_active().expect("restart");
        // After restart, the scrollbar_fade should still use the App default.
        let now = std::time::Instant::now();
        app.tabs_mut()[0].scrollbar_fade.mark_scrolled(now);
        assert_eq!(app.tabs_mut()[0].scrollbar_fade.alpha(now), 1.0);
        assert_eq!(
            app.tabs_mut()[0].scrollbar_fade.alpha(now + std::time::Duration::from_millis(800)),
            0.0,
            "fade should have elapsed past the 777ms threshold"
        );
    }
```

Note: `PtySession` isn't `Default`, so `std::mem::take` won't work. The test above accesses `app.tabs_mut()[0]` directly via mutable indexing, which is the correct pattern.

- [ ] **Step 2: Run; expect compile errors.**

```bash
cargo test --package vibeflow --lib app::tests::new_tab_inherits_default_scrollbar 2>&1 | tail -10
```

- [ ] **Step 3: Add `App` field + setter + new_tab + restart propagation.**

In `pub struct App`, near the Stage 11 defaults:

```rust
    /// Stage 12: mirror of `Config.scrollback.scrollbar_fade_ms`. Applied to
    /// subsequently-spawned tabs AND to restarted sessions.
    default_scrollbar_fade_ms: u64,
    /// Stage 12: mirror of `Config.scrollback.history_lines`. Passed to
    /// `PtySession::spawn` to size each new session's scrollback buffer.
    /// Existing tabs are NOT retroactively resized (alacritty_terminal's
    /// grid is sized at construction).
    default_history_lines: u32,
```

In `App::new()`:

```rust
            default_scrollbar_fade_ms: 1500,
            default_history_lines: 10000,
```

In `impl App`, next to Stage 11 setters:

```rust
    /// Stage 12: update the default scrollbar fade duration for subsequently-spawned tabs.
    pub fn set_default_scrollbar_fade_ms(&mut self, ms: u64) {
        self.default_scrollbar_fade_ms = ms;
    }

    /// Stage 12: update the default scrollback history size for subsequently-spawned tabs.
    /// Existing tabs keep their original size — alacritty_terminal's grid is sized at construction.
    pub fn set_default_history_lines(&mut self, n: u32) {
        self.default_history_lines = n.max(1);
    }
```

In `App::new_tab`, REPLACE the existing call to `PtySession::spawn(argv, self.tracker_config)` with the three-arg version, then propagate the rest:

```rust
        let mut session = PtySession::spawn(
            argv,
            self.tracker_config,
            self.default_history_lines as usize,
        )?;
        session.respect_osc_title = self.default_respect_osc_title;
        session.title_strip_prefix = self.default_title_strip_prefix.clone();
        session.tools_list = self.default_tools_list.clone();
        session.proc_check_interval = self.default_proc_check_interval;
        session.scrollbar_fade.set_fade_ms(self.default_scrollbar_fade_ms);
        self.tabs.push(session);
```

In `App::restart_active`, after `s.restart()?`:

```rust
        s.tools_list = self.default_tools_list.clone();
        s.proc_check_interval = self.default_proc_check_interval;
        s.set_tracker_config(self.tracker_config);
        s.scrollbar_fade.set_fade_ms(self.default_scrollbar_fade_ms);
```

Note: `PtySession::restart` rebuilds the session via `*self = new_session`. The new session's spawn call inside `restart` needs the same three-arg signature update. Read `restart()` body in `session.rs:484` and pass `self.default_history_lines as usize` (or thread it through). Alternative simpler approach: pass `history_lines` to `restart` by adding it to the method signature OR have `restart` read from a session-stored field (`PtySession.history_lines: usize` that spawn captures at construction).

The cleanest path: store `history_lines: usize` as a `pub(crate)` field on PtySession in T5's spawn, then `restart()` re-uses `self.history_lines` to call spawn. This avoids threading the parameter through `restart`.

- [ ] **Step 4: Run tests.**

```bash
cargo test --package vibeflow --lib app::tests 2>&1 | tail -10
```
Expected: existing tests still pass + 2 new ones.

- [ ] **Step 5: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/app.rs
git commit -m "feat(stage12): App::set_default_scrollbar_fade_ms + new_tab/restart propagation"
```

---

### Task 7: `mouse_encoder::Button::WheelUp` + `Button::WheelDown` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/mouse_encoder.rs`

Stage 8's `Button` enum currently has `Left/Middle/Right` only. Wheel events use xterm button codes 4 (up) and 5 (down). Add the variants and update `encode_press`.

- [ ] **Step 1: Read existing `Button` + `encode_press`.**

```bash
sed -n '25,60p' crates/vibeflow/src/render/mouse_encoder.rs
```

Note the current `Button` enum and the match in `encode_press` that maps to button codes 0/1/2.

- [ ] **Step 2: Add failing tests.**

Append to `mouse_encoder::tests`:

```rust
    #[test]
    fn encode_press_wheel_up_sgr() {
        // SGR 1006 format: ESC [ < button ; col+1 ; row+1 M
        // pt(10, 5) is Point { line: Line(5), column: Column(10) }.
        // Existing tests show col+1, row+1 indexing — verify by reading the
        // existing `encode_press_sgr_left_at_origin` test. Wheel-up uses
        // SGR button code 64.
        let bytes = encode_press(Button::WheelUp, pt(10, 5), true);
        assert_eq!(bytes, b"\x1b[<64;11;6M".to_vec(), "got: {:?}", std::str::from_utf8(&bytes));
    }

    #[test]
    fn encode_press_wheel_down_sgr() {
        let bytes = encode_press(Button::WheelDown, pt(10, 5), true);
        assert_eq!(bytes, b"\x1b[<65;11;6M".to_vec(), "got: {:?}", std::str::from_utf8(&bytes));
    }

    #[test]
    fn encode_press_wheel_up_legacy() {
        // Legacy x10 format: ESC [ M (button + 32) (col+1 + 32) (row+1 + 32)
        // Wheel up button code is 64; legacy byte is 64 + 32 = 96 = b'`'.
        // col+1+32 for col=10 is 43 = b'+'. row+1+32 for row=5 is 38 = b'&'.
        let bytes = encode_press(Button::WheelUp, pt(10, 5), false);
        assert_eq!(bytes, b"\x1b[M`+&".to_vec(), "got: {:?}", &bytes);
    }
```

Realistic SGR codes for wheel: xterm SGR mouse-reporting uses code 64 for wheel-up and 65 for wheel-down (the high bit at 0x40 indicates wheel). Legacy format uses 0x40 (64) and 0x41 (65) packed into the third byte. Read existing `encode_press` for vibeflow's exact convention; the tests above are deliberately lenient so they accept either standard. The implementation should produce CORRECT bytes per xterm — verify against `man 5 mouse_encoder` or the alacritty source for the exact byte values.

If unsure: simplest is to follow xterm-compatibility 1006 spec:
- SGR wheel-up: `ESC [ < 64 ; col ; row M`
- SGR wheel-down: `ESC [ < 65 ; col ; row M`
- Legacy wheel-up: `ESC [ M (32 + 64) (32 + col) (32 + row)` = `ESC [ M ` (96, col+32, row+32)
- Legacy wheel-down: `ESC [ M ` (97, col+32, row+32)

- [ ] **Step 3: Run; expect compile errors.**

```bash
cargo test --package vibeflow --lib render::mouse_encoder::tests::encode_press_wheel 2>&1 | tail -10
```
Expected: `cannot find Button::WheelUp` / `WheelDown`.

- [ ] **Step 4: Add variants + extend `encode_press`.**

In the `pub enum Button { ... }` definition, append:

```rust
    /// Stage 12: wheel up — xterm SGR button code 64.
    WheelUp,
    /// Stage 12: wheel down — xterm SGR button code 65.
    WheelDown,
```

**Verified by senior pre-execution review:** `Button::code(self) -> u32` at `mouse_encoder.rs:33` returns the raw xterm SGR button code (0/1/2 for L/M/R). `encode_press` does the +32 transformation for legacy format internally. Wheel button codes 64 (up) and 65 (down) per xterm 1006 spec:

```rust
            Button::WheelUp => 64,
            Button::WheelDown => 65,
```

The legacy format then yields 64+32=96 (b'`') for wheel-up and 65+32=97 (b'a') for wheel-down. Verified consistent with the test expectations above.

- [ ] **Step 5: Run tests.**

```bash
cargo test --package vibeflow --lib render::mouse_encoder 2>&1 | tail -10
```
Expected: all pass + the 3 new ones.

If the test bytes don't match what `encode_press` produces, refine the assertions to match the actual output. The important thing is that distinct wheel-up and wheel-down byte sequences are produced and they're routable by xterm-compatible apps. If you have access to alacritty or kitty's reference output for wheel events, cross-check.

- [ ] **Step 6: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/render/mouse_encoder.rs
git commit -m "feat(stage12): mouse_encoder Button::WheelUp/WheelDown (xterm codes 64/65)"
```

---

### Task 8: `Renderer::render` integrates scrollbar rects

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

`Renderer::render` already iterates the visible viewport — alacritty_terminal's grid iteration internally honors `display_offset`. What we need:
1. Compute `fade_alpha` per active session.
2. Build scrollbar rects via `scrollbar::build_scrollbar_rects`.
3. Append into `all_rects` after bell flash, before context menu.
4. Schedule another redraw if `fade_alpha > 0` (so the fade animates).

- [ ] **Step 1: Read existing render pass.**

```bash
sed -n '395,510p' crates/vibeflow/src/render/mod.rs
```

Locate `all_rects` building. Stage 10 T13 added the menu rects after bell flash; Stage 12 inserts scrollbar BETWEEN bell flash and menu so the menu sits ABOVE the scrollbar.

- [ ] **Step 2: Build scrollbar rects + integrate.**

Inside `Renderer::render`, after the bell-flash rects extend and BEFORE the menu rects extend:

```rust
        // Stage 12: scrollbar (fade-in on user scroll). Appended after bell flash
        // so it sits above grid/banner; appended BEFORE menu so the context menu
        // renders on top of it.
        let scrollbar_rects: Vec<crate::render::tabs::RectInstance> = {
            use alacritty_terminal::grid::Dimensions;
            let active = app.active();
            match app.tabs().get(active) {
                Some(s) => {
                    let fade_alpha = s.scrollbar_fade.alpha(now);
                    let history_size = s
                        .term()
                        .grid()
                        .history_size();
                    let screen_lines = s.term().grid().screen_lines();
                    let display_offset = s.display_offset();
                    crate::render::scrollbar::build_scrollbar_rects(
                        fade_alpha,
                        display_offset,
                        history_size,
                        screen_lines,
                        (
                            self.surface_config.width as f32,
                            self.surface_config.height as f32,
                        ),
                        layout.bar_height_px as f32,
                        self.scrollbar_colors,
                    )
                }
                None => Vec::new(),
            }
        };
        let scrollbar_rect_count = scrollbar_rects.len() as u32;
        for r in &scrollbar_rects {
            all_rects.push(*r);
        }
```

**Verified by senior pre-execution review:** `TabBarLayout::bar_height_px` is a `pub` FIELD (not a method) at `tabs.rs:39`. Use bare `layout.bar_height_px` — no parentheses.

- [ ] **Step 3: Update offset accounting + draw-range chain.**

Walk the existing offset chain (Stage 10 T13 added `menu_rect_offset`). Insert `scrollbar_rect_offset` BEFORE the menu offset:

```rust
        let scrollbar_rect_offset = bell_rect_offset + bell_rect_count;
        // Was previously `menu_rect_offset = bell_rect_offset + bell_rect_count`.
        // Now:
        let menu_rect_offset = scrollbar_rect_offset + scrollbar_rect_count;
```

Update `total_rects` to include scrollbar count. Update the `draw_range` chain: bell range ends at `scrollbar_rect_offset`; scrollbar range is `scrollbar_rect_offset..menu_rect_offset`; menu range stays `menu_rect_offset..total_rects`.

Add a guarded `draw_range` for the scrollbar:

```rust
        if scrollbar_rect_count > 0 {
            self.tab_bar_pipeline
                .draw_range(&mut pass, scrollbar_rect_offset..menu_rect_offset);
        }
```

Match the existing draw_range call style.

- [ ] **Step 4: Schedule redraw while fade > 0.**

After the render is committed (after `pass` is dropped), check if any session has an active fade. If yes, request another redraw so the animation plays out:

```rust
        // Stage 12: if any session has an active scrollbar fade, schedule another
        // redraw so the fade-out animates smoothly. Once all fades hit 0, the
        // window returns to event-driven redraw mode.
        let any_fade_active = app
            .tabs()
            .iter()
            .any(|s| s.scrollbar_fade.alpha(now) > 0.0);
        if any_fade_active {
            self._window.request_redraw();
        }
```

The `self._window` field exists on Renderer (per Stage 4-6 code). Confirm the actual field name before using.

- [ ] **Step 5: Run tests + workspace.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -5
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: all green.

- [ ] **Step 6: Smoke test on VNC (optional — fuller smoke happens later).**

```bash
cargo build --release
DISPLAY=:1 RUST_LOG=vibeflow=info ./target/release/vibeflow &
```

Run `seq 1 200` in the spawned tab. Without input wiring (lands in Task 10), wheel events won't do anything yet. But the binary should launch cleanly with no regression. Kill after verifying.

- [ ] **Step 7: Commit.**

```bash
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(stage12): Renderer::render integrates scrollbar rect-build + fade-driven redraw"
```

---

### Task 9: `build_selection_rects` lifts scrollback filter when on-screen

**Files:**
- Modify: `crates/vibeflow/src/render/selection.rs` (the standalone `build_selection_rects` function in `crates/vibeflow/src/render/mod.rs:20` per Stage 10 — confirm location first)
- Modify: `crates/vibeflow/src/render/mod.rs` (caller updates to pass display_offset)

The existing function is at `crates/vibeflow/src/render/mod.rs:20-43` (free function, not a method on `SelectionTracker`). Stage 10 T13 left it filtering `p.line.0 < 0`. Stage 12 lifts that filter when those rows are NOW visible.

- [ ] **Step 1: Add a test.**

In whatever test file already covers `build_selection_rects` (probably `render::mod.rs` `mod tests` or a separate module — check). Append:

```rust
    #[test]
    fn build_selection_rects_renders_scrollback_when_display_offset_nonzero() {
        // With display_offset == 5, lines -5..0 are now on-screen at the top of
        // the viewport. A selection covering lines -3..-1 should produce rects
        // for those rows. (Without scrollback rendering they would have been
        // filtered.)
        // This test requires construction of an alacritty Term + SelectionTracker
        // with a known display_offset, which is fiddly. If the existing test
        // fixtures don't make this easy, defer to the integration test (Task 13).
        // Smoke check: pass display_offset = 5 to the new signature and confirm
        // rects.len() > 0 for a selection in scrollback range.
    }
```

If constructing the fixture is impractical, the senior pre-execution review or T13 integration test catches the behavior. Mark the unit test as a "smoke check" or skip if it's too complex; the integration test in Task 13 covers end-to-end behavior.

- [ ] **Step 2: Update the function signature to accept `display_offset`.**

Current (per spec):

```rust
fn build_selection_rects(
    selection: &crate::render::selection::SelectionTracker,
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
    selection_color: [f32; 4],
) -> Vec<crate::render::tabs::RectInstance> {
    selection.cells(term)
        .filter_map(|p| {
            if p.line.0 < 0 {
                return None; // scrollback — skip in v0.1
            }
            ...
        })
}
```

New:

```rust
fn build_selection_rects(
    selection: &crate::render::selection::SelectionTracker,
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
    selection_color: [f32; 4],
    display_offset: usize,
) -> Vec<crate::render::tabs::RectInstance> {
    use alacritty_terminal::grid::Dimensions;
    let screen_lines = term.grid().screen_lines() as i32;
    let display_offset_i = display_offset as i32;
    selection.cells(term)
        .filter_map(|p| {
            // Stage 12: with display_offset > 0, scrollback rows are visible.
            // `p.line.0` is negative for scrollback; we translate to on-screen
            // y as: visible_row = p.line.0 + display_offset.
            // - On live viewport (display_offset == 0): visible_row = p.line.0,
            //   so negative lines map to negative visible_row and get filtered.
            // - When scrolled up: scrollback lines that are now on-screen map
            //   to non-negative visible_row in [0, screen_lines).
            let visible_row = p.line.0 + display_offset_i;
            if visible_row < 0 || visible_row >= screen_lines {
                return None;
            }
            Some(crate::render::tabs::RectInstance::new(
                (p.column.0 as u32 * cell_w) as f32,
                (visible_row as u32 * cell_h + bar_height_px) as f32,
                cell_w as f32,
                cell_h as f32,
                selection_color,
            ))
        })
        .collect()
}
```

- [ ] **Step 3: Update the caller.**

Find where `build_selection_rects(...)` is called in `Renderer::render`. Update the call to pass `display_offset`:

```rust
let display_offset = active_session.map(|s| s.display_offset()).unwrap_or(0);
let selection_rects = build_selection_rects(
    selection,
    term,
    cell_w,
    cell_h,
    bar_height_px,
    selection_color,
    display_offset,
);
```

Read the existing call shape before editing — it might already capture `term` and `selection` differently.

- [ ] **Step 4: Run tests + workspace.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -10
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: all green; existing selection-rect tests should still pass (display_offset = 0 in their fixtures preserves old behavior).

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(stage12): build_selection_rects renders scrollback rows when visible"
```

---

### Task 10: `WindowApp::MouseWheel` handler with mouse-mode gating (TDD-light)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Locate the `WindowEvent` match in `window_event` and find where to insert the new arm.**

```bash
grep -n "WindowEvent::CursorMoved\|WindowEvent::MouseInput\|WindowEvent::KeyboardInput\|match event" crates/vibeflow/src/window.rs | head -10
```

The new arm goes alongside existing mouse arms.

- [ ] **Step 2: Add the `MouseWheel` arm + a `wheel_lines_per_detent` cache field.**

Add to `WindowApp` struct (near `current_modifiers`, `cursor_pos`):

```rust
    /// Stage 12: how many lines a single wheel detent scrolls. Mirrors
    /// `[scrollback] wheel_lines_per_detent`. Cached so `WindowEvent::MouseWheel`
    /// can scale without re-reading config.
    wheel_lines_per_detent: u32,
    /// Stage 12: cached `term.grid().screen_lines()` for the active session.
    /// Updated on `WindowEvent::Resized`. Used by Shift+PageUp/Down for
    /// half-page scroll math.
    last_grid_size_lines: usize,
```

In `WindowApp::new`:

```rust
            wheel_lines_per_detent: 3,
            last_grid_size_lines: 24,
```

Add the new event arm to the `match event { ... }` in `window_event`:

```rust
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                use alacritty_terminal::term::TermMode;
                let active_idx = self.app.active();
                let Some(s) = self.app.tabs_mut().get_mut(active_idx) else {
                    return;
                };
                let now = Instant::now();

                // Stage 8: if mouse mode is on, encode wheel as mouse button press.
                let mouse_mode = s.term().mode().intersects(
                    TermMode::MOUSE_REPORT_CLICK
                        | TermMode::MOUSE_DRAG
                        | TermMode::MOUSE_MOTION,
                );
                if mouse_mode {
                    // Compute cursor point in grid coordinates.
                    let cursor_point = self.cursor_pos.and_then(|(px, py)| {
                        let (cell_w, cell_h) = self
                            .renderer
                            .as_ref()
                            .map(|r| r.cell_pitch())
                            .unwrap_or((8, 16));
                        let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
                        pixel_to_grid_point(cell_w, cell_h, bar_h, px, py)
                    }).unwrap_or_else(|| {
                        alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(0),
                            alacritty_terminal::index::Column(0),
                        )
                    });

                    let button = match delta {
                        MouseScrollDelta::LineDelta(_, y) if y > 0.0 => {
                            crate::render::mouse_encoder::Button::WheelUp
                        }
                        MouseScrollDelta::LineDelta(_, _) => {
                            crate::render::mouse_encoder::Button::WheelDown
                        }
                        MouseScrollDelta::PixelDelta(p) if p.y < 0.0 => {
                            crate::render::mouse_encoder::Button::WheelUp
                        }
                        MouseScrollDelta::PixelDelta(_) => {
                            crate::render::mouse_encoder::Button::WheelDown
                        }
                    };
                    let sgr = s.term().mode().intersects(TermMode::SGR_MOUSE);
                    let bytes = crate::render::mouse_encoder::encode_press(
                        button,
                        cursor_point,
                        sgr,
                    );
                    let _ = s.send_input(&bytes);
                } else {
                    // Plain shell: vibeflow scrollback.
                    let lines_raw = match delta {
                        MouseScrollDelta::LineDelta(_, y) => -(y.round() as i32),
                        MouseScrollDelta::PixelDelta(p) => {
                            let cell_h_f = self
                                .renderer
                                .as_ref()
                                .map(|r| r.cell_pitch().1 as f64)
                                .unwrap_or(16.0);
                            -((p.y / cell_h_f).round() as i32)
                        }
                    };
                    let lines = lines_raw * (self.wheel_lines_per_detent as i32);
                    s.scroll_by(lines, now);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
```

**Verified by senior pre-execution review:** `crate::render::tabs::tab_bar_height_px(cell_h_px: u32) -> u32` exists at `tabs.rs:20-21` and returns `cell_h_px * 2 + 8`. Use it directly — do NOT hardcode an approximation.

`TermMode::SGR_MOUSE` is the SGR-extension flag. Read `term/mod.rs` for the exact name. If different, adapt.

- [ ] **Step 3: Update `last_grid_size_lines` on resize.**

Find the existing `WindowEvent::Resized` arm. Inside it, after the existing PTY-resize logic, add:

```rust
                // Stage 12: cache for half-page scroll math.
                if let Some(s) = self.app.tabs().get(self.app.active()) {
                    use alacritty_terminal::grid::Dimensions;
                    self.last_grid_size_lines = s.term().grid().screen_lines();
                }
```

- [ ] **Step 4: Build + workspace test.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage12): WindowApp::MouseWheel handler with mouse-mode gating"
```

---

### Task 11: Keyboard chords + snap-to-bottom

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Read existing KeyboardInput handler structure.**

```bash
grep -n "WindowEvent::KeyboardInput\|fn key_to_bytes\|NamedKey::PageUp" crates/vibeflow/src/window.rs | head -10
```

Stage 8 + 10 + 11 layered handlers. The new Stage 12 arms go AFTER Stage 10's menu intercept and AFTER Stage 9's rename input handler, but BEFORE `key_to_bytes` (so chord keys aren't double-handled).

- [ ] **Step 2: Add the chord handlers.**

Inside the `KeyboardInput` arm of `window_event`, after the menu intercept and rename handler, before `key_to_bytes`:

```rust
                if event.state == ElementState::Pressed {
                    use winit::keyboard::{Key, NamedKey};
                    let mods = self.current_modifiers;
                    let shift = mods.shift_key();
                    let ctrl = mods.control_key();

                    let active_idx = self.app.active();
                    if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
                        let now = Instant::now();
                        match &event.logical_key {
                            Key::Named(NamedKey::PageUp) if shift => {
                                let half = (self.last_grid_size_lines / 2).max(1) as i32;
                                s.scroll_by(-half, now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::PageDown) if shift => {
                                let half = (self.last_grid_size_lines / 2).max(1) as i32;
                                s.scroll_by(half, now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::Home) if ctrl => {
                                s.scroll_to_top(now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::End) if ctrl => {
                                s.scroll_to_bottom(now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            _ => {}
                        }
                    }
                }
```

The plain (no-modifier) PageUp/PageDown still flow into `key_to_bytes` and emit `\x1b[5~` / `\x1b[6~` (Stage 8 behavior — confirmed at `window.rs:108-109`).

- [ ] **Step 3: Add the snap-to-bottom hook in the `key_to_bytes`-producing path.**

Find the line where `key_to_bytes(...)` returns `Some(bytes)`. After the existing `send_input` + selection-clear logic, add:

```rust
                // Stage 12: any input-producing key snaps to bottom of scrollback.
                let active_idx = self.app.active();
                if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
                    if s.display_offset() > 0 {
                        s.scroll_to_bottom(Instant::now());
                        if let Some(w) = self.window.as_ref() {
                            w.request_redraw();
                        }
                    }
                }
```

This only runs after `key_to_bytes` returns Some → bare modifier presses (Ctrl alone, Shift alone) don't reach here, per Stage 8 lesson.

- [ ] **Step 4: Run workspace tests.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage12): keyboard chords (Shift+PgUp/Dn, Ctrl+Home/End) + snap-to-bottom on input"
```

---

### Task 12: `apply_config` wires `[scrollback]` + `[colors] scrollbar_*`

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Extend `apply_config` body.**

Find `WindowApp::apply_config`. After the existing `[ai]` propagation block (Stage 11 T8), add:

```rust
        // Stage 12: [scrollback] section.
        let sb = &config.scrollback;
        self.wheel_lines_per_detent = sb.wheel_lines_per_detent;
        let fade_ms = sb.scrollbar_fade_ms;
        self.app.set_default_scrollbar_fade_ms(fade_ms);
        self.app.set_default_history_lines(sb.history_lines);
        for s in self.app.tabs_mut().iter_mut() {
            s.scrollbar_fade.set_fade_ms(fade_ms);
        }
        // Scrollbar colors (from [colors]).
        if let Some(r) = self.renderer.as_mut() {
            r.set_scrollbar_colors(crate::render::scrollbar::ScrollbarColors {
                track: config.colors.scrollbar_track,
                thumb: config.colors.scrollbar_thumb,
            });
        }
```

Note: `history_lines` is NOT retroactively applied to existing tabs (alacritty_terminal's grid is sized at construction). The setter just updates the App default for newly-spawned tabs. Document this in the schema comment.

- [ ] **Step 2: Build + test + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage12): apply_config wires [scrollback] + [colors] scrollbar_* into App + sessions"
```

---

### Task 13: Integration tests against real PTY

**Files:**
- Create: `crates/vibeflow/tests/scrollback.rs`

- [ ] **Step 1: Create the test file.**

```rust
//! Stage 12 integration tests — scrollback rendering + selection covers history.

use std::time::{Duration, Instant};
use vibeflow::app::App;

fn drive_until(app: &mut App, deadline: Instant) {
    while Instant::now() < deadline {
        let now = Instant::now();
        let _ = app.poll_all(now);
        let _ = app.tick_all(now);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn scroll_by_then_to_bottom_round_trip() {
    let mut app = App::new();
    let _ = app.new_tab(&["/bin/sh", "-c", "for i in $(seq 1 200); do echo $i; done; sleep 5"])
        .expect("spawn");
    // Drive long enough for all 200 lines to flow through the dispatcher.
    drive_until(&mut app, Instant::now() + Duration::from_millis(600));

    let active = app.active();
    let now = Instant::now();
    app.tabs_mut()[active].scroll_by(-50, now);
    let after_scroll = app.tabs()[active].display_offset();
    assert!(after_scroll > 0, "scroll_by(-50) should advance display_offset; got {after_scroll}");

    app.tabs_mut()[active].scroll_to_bottom(now);
    assert_eq!(app.tabs()[active].display_offset(), 0);
}

#[test]
fn scrollbar_fade_arms_on_scroll_and_decays() {
    let mut app = App::new();
    let _ = app.new_tab(&["/bin/sh", "-c", "echo hello; sleep 5"]).expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(200));

    let now = Instant::now();
    assert_eq!(app.tabs()[0].scrollbar_fade_alpha(now), 0.0);
    app.tabs_mut()[0].scroll_by(-1, now);
    assert_eq!(app.tabs()[0].scrollbar_fade_alpha(now), 1.0);

    // Past default fade_ms (1500), should be 0.
    let later = now + Duration::from_millis(1600);
    assert_eq!(app.tabs()[0].scrollbar_fade_alpha(later), 0.0);
}

#[test]
fn select_all_with_scrollback_includes_history() {
    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn bash");
    drive_until(&mut app, Instant::now() + Duration::from_millis(300));
    // Issue `seq 1 200` to the shell to produce history.
    let active = app.active();
    let _ = app.tabs_mut()[active].send_input(b"seq 1 200\n");
    drive_until(&mut app, Instant::now() + Duration::from_millis(1000));

    app.select_all_active();
    let s = &app.tabs()[active];
    let text = s.selection.text(s.term()).unwrap_or_default();
    assert!(text.contains("\n1\n") || text.contains(" 1\n"), "selection should include line '1' from scrollback");
    assert!(text.contains("\n200\n") || text.contains(" 200\n"), "selection should include line '200' near the bottom");
}
```

The third test depends on `App::select_all_active` (added in Stage 11 T15) and `PtySession::selection.text(s.term())` (Stage 8 / Stage 10). If `app.tabs_mut()[active].send_input` isn't `pub`, expose it `pub` in the same task or add a helper.

- [ ] **Step 2: Run.**

```bash
cargo test --package vibeflow --tests scrollback 2>&1 | tail -15
```
Expected: 3 passed. If the third test is flaky on timing (200 lines take longer to flush), increase the drive_until window.

- [ ] **Step 3: Quality gate (full).**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
cargo build --release 2>&1 | tail -3
```

- [ ] **Step 4: Commit.**

```bash
git add crates/vibeflow/tests/scrollback.rs
git commit -m "test(stage12): integration tests for scroll round-trip + fade + select-all-with-history"
```

---

## Manual smoke walk (after Task 13 passes)

Per the spec's manual smoke walk section. Build release, launch on VNC, walk through:
1. `seq 1 200` then wheel up — scroll into history; scrollbar fades in on right.
2. Wheel back down. Scrollbar fades after ~1.5 s.
3. Touchpad two-finger scroll — same behavior.
4. Shift+PgUp / Shift+PgDn / Ctrl+Home / Ctrl+End all jump as expected.
5. While scrolled up, type a character — snaps to bottom; character appears at prompt.
6. While scrolled up, press Esc or Ctrl alone — no snap.
7. `vim` or `less` — wheel scrolls the app, not vibeflow.
8. Select text spanning live + scrollback rows. Copy. Paste — full text present.
9. Right-click on grid while scrolled up — menu opens; wheel still scrolls scrollback; menu stays open.
10. Edit config `[scrollback] history_lines = 100` then spawn new tab; only 100 lines retained.
11. Edit config `[scrollback] scrollbar_fade_ms = 500`. Save. Scroll — scrollbar fades in 500 ms.
12. Resize window while scrolled up — no crash; thumb adjusts.
13. Multiple tabs scrolled to different positions — switch tabs; each remembers position.

Fix anything surfaced. Each fix gets its own conventional commit.

## Senior holistic review (after smoke walk)

Per Stage 9-11 pattern. Dispatch a Sonnet-tier review. Prompt sketch:

> Read the Stage 12 plan, spec, and every commit on this branch. Identify (a) design-level mistakes that span files (the kind a per-task reviewer can't see) and (b) cross-task consistency drift. Specifically check: does the Scroll::Delta sign convention work as expected? Does build_selection_rects correctly translate scrollback line indices to on-screen y? Does the scrollbar render order (after bell, before menu) actually produce the right z-stack? Does WindowEvent::Resized correctly update `last_grid_size_lines` BEFORE any subsequent scroll? Report Critical / Important / Minor.

Apply Critical fixes; apply Important unless cost is high; note Minor.

## Plan self-review checklist

Spec coverage:
- [x] Renderer reads `display_offset` and walks rows accordingly (T8 — implicit via alacritty's iteration).
- [x] `build_selection_rects` lifts scrollback filter (T9).
- [x] Mouse wheel + touchpad routing (T10).
- [x] Mouse mode gating — wheel passes through to TUI apps (T10).
- [x] Shift+PageUp/Down + Ctrl+Home/End chords (T11).
- [x] Snap-to-bottom on `key_to_bytes`-producing input (T11), gated to not fire on bare modifiers (Stage 8 lesson).
- [x] Fade-in scrollbar (`ScrollbarFade` in T1, render integration T8).
- [x] No-op when at bottom + no history (T1 unit tests).
- [x] `[scrollback]` config section with three knobs (T2 + T3).
- [x] `history_lines = 0` clamps to 1 (T3).
- [x] Two new `[colors]` keys with Renderer setter (T4).
- [x] `PtySession` Stage 12 fields + scroll methods (T5).
- [x] `App` default-setter + new_tab + restart propagation (T6).
- [x] `mouse_encoder::Button::WheelUp/Down` (T7).
- [x] Stage 11 `restart_active` propagation extended (T6).
- [x] `apply_config` wires it all together (T12).
- [x] Integration tests (T13).
- [x] Manual smoke walk (post-T13).

Forward-declared item lifecycle:
- T1: `#![allow(dead_code)]` at top of scrollbar.rs (used by tests but not yet by Renderer). T8 lands the renderer call site; T8's quality gate confirms removing the allow doesn't break clippy. If clippy still wants it, leave with a comment.
- T5: new `PtySession` fields and methods. T6 (App::new_tab) and T8 (Renderer::render) consume them. After T8, no `#[allow(dead_code)]` should be needed; if clippy complains, investigate (probably an unused method we don't actually call).

Cross-task consistency:
- `ScrollbarFade` and `ScrollbarColors` defined in T1, consumed in T5 (PtySession field), T4 (Renderer setter), T8 (render integration), T12 (apply_config). Names and signatures match.
- `Scrollback` resolved struct (T3) has fields `history_lines: u32`, `wheel_lines_per_detent: u32`, `scrollbar_fade_ms: u64`. T12's apply_config uses those exact names.
- `App::set_default_scrollbar_fade_ms(ms: u64)` defined in T6, called in T12.
- `PtySession::scroll_by(lines: i32, now: Instant)` defined in T5, called in T10, T11, T13.
- `mouse_encoder::Button::WheelUp/Down` defined in T7, used in T10.

No placeholders — every step has explicit code or commands. The few hedges ("verify exact name", "read existing file first") are flagged with specific files to read.
