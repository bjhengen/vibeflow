# vibeflow — Stage 12: Scrollback rendering

**Status:** Draft, pending review
**Date:** 2026-05-13
**Author:** brainstormed with Claude

## Summary

Stage 12 makes scrollback history visible. Previous stages built the data-model side incrementally — `alacritty_terminal::Term` maintains the history grid, Stage 10's `SelectionTracker::cells()` and `text()` already iterate the full buffer (including negative line indices), Stage 10's `Shortcut::SelectAll` covers the whole buffer. What was missing: the renderer only walked the visible viewport, and `build_selection_rects` filtered out scrollback rows. Stage 12 closes that gap by reading `Term::grid().display_offset()` per frame, walking the appropriate row range, and lifting the selection-rect filter when scrollback rows are on-screen.

The user-facing surface is mouse-wheel scrolling, touchpad two-finger scrolling, keyboard chords (Shift+PageUp/Down, Ctrl+Home/End), a fade-in scrollbar on the right edge, and a snap-to-bottom rule on user keystrokes. Mouse-wheel events in TUI mouse-mode apps (vim, less, htop) pass through to the app via Stage 8's `mouse_encoder` rather than scrolling vibeflow's history. New `[scrollback]` config section with three knobs (`history_lines`, `wheel_lines_per_detent`, `scrollbar_fade_ms`) and two new `[colors]` keys (`scrollbar_track`, `scrollbar_thumb`).

## Goals & Non-Goals

### Goals

- User can scroll back through PTY history with mouse wheel, touchpad two-finger, Shift+PageUp/Down, and Ctrl+Home/End.
- Scrollbar is invisible at rest; fades in for ~1.5 s after any user scroll activity; fades back to invisible.
- New output that arrives while user is scrolled up does NOT change the scroll position and does NOT pop the scrollbar back into visibility.
- Any user keystroke that produces input bytes snaps the viewport back to bottom. Bare modifier presses do not snap (per Stage 8's modifier-press lesson).
- Mouse-wheel events in mouse-mode TUI apps (alt-screen on, mouse-reporting enabled) pass through to the app via the existing `mouse_encoder` path. Plain shells get scrollback.
- Selection that spans scrollback + live viewport renders correctly: rects show for rows currently on-screen; rows still off-screen stay filtered. Selection-text-extraction already worked from Stage 10.
- `[scrollback] history_lines` defaults to 10,000 (alacritty_terminal default); user-configurable. Applied at tab spawn (changing the value affects new tabs only).
- All proc-reading code from Stage 11 still works; tab indicators (Stage 4-11 visual treatment) unaffected.

### Non-Goals (Stage 12)

- Block (column) selection — Alt+drag. Stage 13 polish.
- Indicator visual prominence redesign. Stage 13 polish, user-prioritized for that stage.
- Shift / Ctrl modifier arrow keys (word/line jump). Stage 13 polish.
- Bell behavior config. Stage 13 polish.
- Font priority live-reload. Stage 13 polish.
- iTerm2 color-scheme import. Stage 13 polish.
- "More content below" status chip when scrolled up. Considered and explicitly rejected — quietest UX wins; user discovers new content by scrolling back down.
- macOS/Windows scroll behavior tuning. v0.1 platform list is Linux-only; cross-platform-clean code is preserved but not tuned.
- Search-in-scrollback (`/`-style search). Explicitly out of v0.1 scope per the original design spec.
- Splits/panes. Explicitly out of v0.1 scope.

## Architecture

### Mechanism

`alacritty_terminal::Term` already maintains the scrollback grid via its internal `Grid<Cell>`. The crate exposes (verified against `alacritty_terminal-0.24.2/src/grid/mod.rs` and `term/mod.rs`):
- `term.grid().display_offset() -> usize` — how many lines into history the viewport is scrolled.
- `term.scroll_display(scroll: Scroll)` at `term/mod.rs:389` — mutates `display_offset`. The `Scroll` enum at `grid/mod.rs:73` has exactly five variants: `Delta(i32)`, `PageUp`, `PageDown`, `Top`, `Bottom`. **No `Lines(i32)` variant.** Sign convention for `Delta`: needs runtime verification during implementation (use a tiny test: seed history, call `Scroll::Delta(1)` and check whether `display_offset` increases or decreases).
- `term.grid()` also exposes `history_size()` and `screen_lines()` via the `Dimensions` trait already in scope (Stage 11 used these for `SelectionTracker::select_all`).

Stage 12 leverages this directly — no parallel buffer, no custom scrollback storage, no fork of the crate. The state that's NEW in vibeflow:
- `PtySession.scrollbar_fade: ScrollbarFade` — per-session fade-state for the scrollbar thumb visibility.
- The wheel-event routing decision (in-mouse-mode → existing Stage 8 `mouse_encoder` path; else → `scroll_display`).
- Keyboard chord plumbing for Shift+PgUp/Dn and Ctrl+Home/End.

I considered rolling our own scrollback display layer (manually iterating grid rows past `display_offset` in vibeflow code rather than via `Term`). Rejected: reinvents what alacritty already does correctly and would diverge from upstream behavior on edge cases (resize, alt-screen toggle, clear-history sequences).

### Module layout

| File | Status | Responsibility |
|---|---|---|
| `crates/vibeflow/src/render/scrollbar.rs` | NEW | `ScrollbarFade` (per-session state machine for thumb visibility); `build_scrollbar_rects` (pure function emitting track + thumb rects given current state); `ScrollbarColors` cache struct. Pure logic, no wgpu. |
| `crates/vibeflow/src/render/mod.rs` | TOUCHED | `Renderer::render` walks rows accounting for `display_offset`; appends `build_scrollbar_rects` output into `all_rects` after bell-flash but BEFORE context-menu; schedules redraw while fade_alpha > 0. New `Renderer::set_scrollbar_colors` setter mirroring Stage 9's pattern. |
| `crates/vibeflow/src/render/selection.rs` | TOUCHED (light) | `build_selection_rects` accepts `display_offset` and translates negative-line scrollback indices to on-screen y when they're now visible. Off-screen rows still filtered. |
| `crates/vibeflow/src/session/session.rs` | TOUCHED | `PtySession` gains `scrollbar_fade: ScrollbarFade`. New `scroll_by(lines: i32, now: Instant)` method (negative = into history, positive = toward live); new `scroll_to_bottom(now: Instant)` method; new `scroll_to_top(now: Instant)` method; new `display_offset() -> usize` accessor (read-only convenience). `restart()` resets `scrollbar_fade` and the underlying `Term`'s display_offset. |
| `crates/vibeflow/src/window.rs` | TOUCHED | New `WindowEvent::MouseWheel` arm with mouse-mode gating. New chord handlers in `KeyboardInput`: `Shift+PageUp`, `Shift+PageDown`, `Ctrl+Home`, `Ctrl+End`. Snap-to-bottom on any key whose `key_to_bytes` returns `Some` (gated to avoid bare modifier presses snapping). `apply_config` wires `[scrollback]` knobs into App + per-session updates. WindowApp gains a cached `last_grid_size_lines: usize` so half-page scroll knows the current viewport height. |
| `crates/vibeflow/src/render/mouse_encoder.rs` | TOUCHED | Add `Button::WheelUp` (xterm code 4) and `Button::WheelDown` (xterm code 5) variants; update `encode_press`'s match to emit codes 4 and 5; add unit tests for both. Stage 12 wheel-in-mouse-mode routing depends on these. |
| `crates/vibeflow/src/app.rs` | TOUCHED (light) | `App::restart_active` propagates the new `scrollbar_fade` defaults (similar to Stage 11's tools_list propagation). `App` gains `default_scrollbar_fade_ms` field + `set_default_scrollbar_fade_ms` setter mirroring Stage 9's pattern. |
| `crates/vibeflow/src/config/{schema,mod}.rs` | TOUCHED | New `[scrollback]` section with `history_lines` (u32, default 10000), `wheel_lines_per_detent` (u32, default 3), `scrollbar_fade_ms` (u64, default 1500). Two new `[colors]` keys (`scrollbar_track`, `scrollbar_thumb`). Mirror Stage 11 `[ai]` and Stage 9 `[colors]` patterns. |
| `crates/vibeflow/tests/scrollback.rs` | NEW | Linux-friendly integration tests against a real PTY. |

### Render order per frame

```
clear → grid cells (now rows = visible viewport starting from display_offset)
      → cursor (suppressed when display_offset > 0; cursor isn't meaningful in scrollback)
      → tab strip → bell flash → SCROLLBAR → context menu
```

Scrollback rendered before context menu so the menu sits on top. Cursor is hidden during scrollback because the live cursor position isn't where the user is looking; matches Alacritty/Kitty behavior.

### Data flow

Per frame:
1. `Renderer::render` queries `session.term.grid().display_offset()` and `history_size()` for the active tab.
2. Cell-render pass iterates rows accounting for `display_offset` (alacritty's grid iteration already supports this).
3. Selection-rect builder receives `display_offset`; emits rects for any selected rows currently visible, filters off-screen ones.
4. Scrollbar builder receives current `fade_alpha = scrollbar_fade.alpha(now)`. Empty Vec when `fade_alpha == 0`. Otherwise track + thumb rects appended to `all_rects` after bell-flash.
5. If `fade_alpha > 0`, schedule another redraw (otherwise the fade-out wouldn't visibly animate — vibeflow only redraws on dirty events).

On user input:
- Mouse wheel: check `term.mode()` for mouse-reporting flags. If set, encode wheel as mouse button via `mouse_encoder::encode_wheel` and write to PTY. Else, call `session.scroll_by(delta * wheel_lines_per_detent, now)`.
- Keyboard chord (Shift+PgUp etc.): direct `scroll_display` call + `scrollbar_fade.mark_scrolled(now)`.
- Keyboard chord that produces `key_to_bytes(...) == Some(bytes)`: write to PTY AND `if display_offset > 0 { scroll_to_bottom + brief fade }`. The fade lets the user see "yes, we moved" rather than the viewport jumping silently.

## Components

### `ScrollbarFade`

```rust
//! Per-session fade-state for the scrollbar thumb. Stays at α=0 when the
//! session hasn't been scrolled recently; pops to α=1 the instant a scroll
//! happens; fades linearly back to 0 over `fade_ms` after the last activity.

#[derive(Debug, Clone)]
pub struct ScrollbarFade {
    /// Wall-clock time of most recent scroll input. None means never scrolled.
    last_scroll_at: Option<Instant>,
    /// Full-fade duration after last activity. Defaults to 1500 ms via Config.
    fade_ms: u64,
}

impl ScrollbarFade {
    pub fn new(fade_ms: u64) -> Self {
        Self { last_scroll_at: None, fade_ms }
    }

    pub fn mark_scrolled(&mut self, now: Instant) {
        self.last_scroll_at = Some(now);
    }

    /// 1.0 at the instant of activity, linearly decreasing to 0.0 at fade_ms.
    /// Returns 0.0 if never scrolled or fade has elapsed.
    pub fn alpha(&self, now: Instant) -> f32 {
        let Some(last) = self.last_scroll_at else { return 0.0; };
        let elapsed = now.saturating_duration_since(last);
        let elapsed_ms = elapsed.as_millis() as u64;
        if elapsed_ms >= self.fade_ms {
            0.0
        } else {
            1.0 - (elapsed_ms as f32 / self.fade_ms as f32)
        }
    }

    /// Update the fade duration at runtime. Existing fade-in-progress keeps
    /// its baseline (last_scroll_at) and is re-evaluated on next alpha() call.
    pub fn set_fade_ms(&mut self, fade_ms: u64) {
        self.fade_ms = fade_ms;
    }
}
```

### `build_scrollbar_rects`

```rust
pub fn build_scrollbar_rects(
    fade_alpha: f32,
    display_offset: usize,
    history_size: usize,
    screen_lines: usize,
    surface_size: (f32, f32),  // physical px
    bar_height_px: f32,         // tab strip is above the scrollbar
    colors: ScrollbarColors,
) -> Vec<crate::render::tabs::RectInstance> {
    // Bail early when there's nothing to show.
    if fade_alpha == 0.0 {
        return Vec::new();
    }
    if display_offset == 0 && history_size == 0 {
        return Vec::new();
    }

    const TRACK_WIDTH_PX: f32 = 8.0;
    const THUMB_MIN_HEIGHT_PX: f32 = 20.0;
    const TRACK_INSET_PX: f32 = 1.0;  // thumb is slightly narrower than track

    let (surface_w, surface_h) = surface_size;
    let track_x = surface_w - TRACK_WIDTH_PX;
    let track_y = bar_height_px;
    let track_h = surface_h - bar_height_px;

    // Thumb position: maps display_offset (0 = bottom, history_size = top) to
    // a y-coordinate in the track.
    let total_lines = (history_size + screen_lines).max(1) as f32;
    let visible_lines = screen_lines as f32;
    let thumb_h = (visible_lines / total_lines * track_h).max(THUMB_MIN_HEIGHT_PX).min(track_h);
    let max_thumb_y_offset = track_h - thumb_h;
    let scroll_fraction = (display_offset as f32 / history_size.max(1) as f32).min(1.0);
    // display_offset = 0 → thumb at bottom; display_offset = history_size → thumb at top.
    let thumb_y = track_y + max_thumb_y_offset * (1.0 - scroll_fraction);

    let track_color = scale_alpha(colors.track, fade_alpha);
    let thumb_color = scale_alpha(colors.thumb, fade_alpha);

    vec![
        crate::render::tabs::RectInstance::new(track_x, track_y, TRACK_WIDTH_PX, track_h, track_color),
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
```

### `ScrollbarColors`

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarColors {
    pub track: [f32; 4],
    pub thumb: [f32; 4],
}

impl Default for ScrollbarColors {
    fn default() -> Self {
        Self {
            track: [1.0, 1.0, 1.0, 0.04],   // very faint white at α=0.04
            thumb: [1.0, 1.0, 1.0, 0.22],   // visible-but-quiet white at α=0.22
        }
    }
}
```

`Renderer.scrollbar_colors: ScrollbarColors` field added, `set_scrollbar_colors` setter, `apply_config` writes from `[colors]` schema fields `scrollbar_track` / `scrollbar_thumb`.

### `PtySession::scroll_*` methods

```rust
impl PtySession {
    /// Scroll the display by `lines`. Negative = into history; positive = toward live.
    /// Marks scrollbar fade timer. No-op when `lines == 0`.
    pub fn scroll_by(&mut self, lines: i32, now: Instant) {
        use alacritty_terminal::grid::Scroll;
        if lines == 0 { return; }
        // alacritty's Scroll::Delta convention: positive = scroll down (toward live).
        // vibeflow's `scroll_by` convention: negative = into history.
        // Net: pass -lines to alacritty. (Verify during impl — easy to invert.)
        self.term.scroll_display(Scroll::Delta(-lines));
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Jump to top of history.
    pub fn scroll_to_top(&mut self, now: Instant) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Top);
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Jump back to live viewport.
    pub fn scroll_to_bottom(&mut self, now: Instant) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Bottom);
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Read current display_offset. 0 = at live viewport. Used by window.rs
    /// and the renderer.
    pub fn display_offset(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        self.term.grid().display_offset()
    }
}
```

### `WindowApp::handle_mouse_wheel`

New event arm in `window_event`:
```rust
WindowEvent::MouseWheel { delta, .. } => {
    let active_idx = self.app.active();
    let Some(s) = self.app.tabs_mut().get_mut(active_idx) else { return; };
    let now = Instant::now();

    // Stage 8: if the active session has mouse mode enabled (TUI in alt-screen),
    // encode the wheel as a mouse button event and pass to the PTY. Stage 12
    // scrollback stays inactive.
    use alacritty_terminal::term::TermMode;
    let mouse_mode = s.term().mode().intersects(
        TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
    );
    if mouse_mode {
        // Stage 8's `mouse_encoder` (`render::mouse_encoder.rs:26`) currently
        // has Button::{Left, Middle, Right} only — no WheelUp/WheelDown
        // variants and no `encode_wheel` function. Stage 12 MUST extend
        // mouse_encoder with:
        //   - new variants Button::WheelUp (xterm button code 4) and
        //     Button::WheelDown (xterm button code 5);
        //   - the existing `encode_press` matches on the variant and emits
        //     the correct button code (4 for WheelUp, 5 for WheelDown);
        //   - OR a new `encode_wheel(direction, point, sgr)` helper that
        //     calls into encode_press with the appropriate Button variant.
        // The plan should pin this down. Reading mouse_encoder.rs in full
        // is required before writing the wheel handler.
        let cursor_point = self.cursor_pos
            .and_then(|(px, py)| {
                let (cw, ch) = self.renderer.as_ref()?.cell_pitch();
                let bar_h = /* tab strip height from renderer */;
                pixel_to_grid_point(cw, ch, bar_h, px, py)
            })
            .unwrap_or_else(|| alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(0),
                alacritty_terminal::index::Column(0),
            ));
        let button = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) if y > 0.0 => crate::render::mouse_encoder::Button::WheelUp,
            winit::event::MouseScrollDelta::LineDelta(_, _) => crate::render::mouse_encoder::Button::WheelDown,
            winit::event::MouseScrollDelta::PixelDelta(p) if p.y < 0.0 => crate::render::mouse_encoder::Button::WheelUp,
            winit::event::MouseScrollDelta::PixelDelta(_) => crate::render::mouse_encoder::Button::WheelDown,
        };
        let bytes = crate::render::mouse_encoder::encode_press(button, cursor_point, /* sgr */ true);
        let _ = s.send_input(&bytes);
    } else {
        // Plain shell: vibeflow scrollback.
        let lines = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => -(y.round() as i32),
            winit::event::MouseScrollDelta::PixelDelta(p) => {
                let cell_h = self.renderer.as_ref().map(|r| r.cell_pitch().1).unwrap_or(16);
                -((p.y / cell_h as f64).round() as i32)
            }
        };
        let lines = lines * (self.wheel_lines_per_detent as i32);
        s.scroll_by(lines, now);
    }
    if let Some(w) = self.window.as_ref() { w.request_redraw(); }
}
```

`mouse_encoder::encode_wheel` is the helper added in Stage 8. If the signature differs slightly, adapt — the senior pre-execution review will catch mismatches.

### `WindowApp::KeyboardInput` additions

In the existing key-handling chain (after the menu-open intercept from Stage 10, after the rename-input handler from Stage 9, before/around `key_to_bytes`):

```rust
use winit::keyboard::{Key, NamedKey, ModifiersState};
let shift = self.current_modifiers.shift_key();
let ctrl  = self.current_modifiers.control_key();

if event.state == ElementState::Pressed {
    let active_idx = self.app.active();
    let Some(s) = self.app.tabs_mut().get_mut(active_idx) else { /* fall through */ };
    let now = Instant::now();
    match &event.logical_key {
        Key::Named(NamedKey::PageUp)   if shift => { /* half-page up */
            let half = (self.last_grid_size_lines / 2).max(1) as i32;
            s.scroll_by(-half, now);
            if let Some(w) = self.window.as_ref() { w.request_redraw(); }
            return;
        }
        Key::Named(NamedKey::PageDown) if shift => { /* half-page down */
            let half = (self.last_grid_size_lines / 2).max(1) as i32;
            s.scroll_by(half, now);
            if let Some(w) = self.window.as_ref() { w.request_redraw(); }
            return;
        }
        Key::Named(NamedKey::Home) if ctrl => {
            s.scroll_to_top(now);
            if let Some(w) = self.window.as_ref() { w.request_redraw(); }
            return;
        }
        Key::Named(NamedKey::End) if ctrl => {
            s.scroll_to_bottom(now);
            if let Some(w) = self.window.as_ref() { w.request_redraw(); }
            return;
        }
        _ => {}
    }
}
```

`self.last_grid_size_lines` is a cached `usize` of the current grid's screen_lines (updated on resize). If WindowApp doesn't already cache this, add it (it's also useful for the wheel-by-cell-height calculation).

The placement matters: these arms go AFTER the menu-open intercept (so Shift+PageUp while a menu is open doesn't scroll the grid — the menu's own focus-nav handler runs first) and BEFORE `key_to_bytes` (so we don't accidentally also send escape sequences for these chords). Stage 8's existing Page Up/Down handling (if any) needs the shift-guard so plain PageUp still works.

### Snap-to-bottom hook

In the existing `key_to_bytes` branch:
```rust
if let Some(bytes) = key_to_bytes(&event.logical_key, event.state, self.current_modifiers) {
    // ...existing send_input + selection-clear logic...

    // Stage 12: user is interacting with live session — snap to bottom.
    let active_idx = self.app.active();
    if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
        if s.display_offset() > 0 {
            s.scroll_to_bottom(Instant::now());
        }
    }
}
```

Piggybacks on `key_to_bytes` returning `Some` — bare modifier presses don't reach this path (per Stage 8 lesson). So Ctrl-alone, Shift-alone, Alt-alone don't snap.

### Config schema

`config/schema.rs`:
```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrollbackSection {
    pub history_lines: Option<u32>,
    pub wheel_lines_per_detent: Option<u32>,
    pub scrollbar_fade_ms: Option<u64>,
}
```

Plus `pub scrollback: Option<ScrollbackSection>` field on `ConfigFile` (the actual struct name verified in Stage 11).

`config/mod.rs`:
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Scrollback {
    pub history_lines: u32,
    pub wheel_lines_per_detent: u32,
    pub scrollbar_fade_ms: u64,
}
```

Defaults: `history_lines = 10000`, `wheel_lines_per_detent = 3`, `scrollbar_fade_ms = 1500`.

`apply_scrollback(schema, &mut resolved)` mirrors Stage 11's `apply_ai`. All fields infallible (no parsing errors).

`[colors]` adds two keys: `scrollbar_track` and `scrollbar_thumb` (Stage 9 hex-string pattern). Defaults match `ScrollbarColors::default`.

### `apply_config` wiring

In `WindowApp::apply_config`, after the `[ai]` block:
```rust
// Stage 12: [scrollback] section.
let sb = &config.scrollback;
self.wheel_lines_per_detent = sb.wheel_lines_per_detent;
let fade_ms = sb.scrollbar_fade_ms;
self.app.set_default_scrollbar_fade_ms(fade_ms);
for s in self.app.tabs_mut().iter_mut() {
    s.scrollbar_fade.set_fade_ms(fade_ms);
}
// scrollbar colors propagate to renderer.
if let Some(r) = self.renderer.as_mut() {
    r.set_scrollbar_colors(crate::render::scrollbar::ScrollbarColors {
        track: config.colors.scrollbar_track,
        thumb: config.colors.scrollbar_thumb,
    });
}
```

Note: `history_lines` is NOT propagated to existing tabs (alacritty_terminal's grid is sized at construction; mid-life resize of the history buffer isn't supported by the crate without significant work). The setting applies to NEW tabs only via `App::new_tab`'s spawn config. Document this in the config comment.

## Edge cases

- **Tab resize while scrolled up.** `Term::resize` clamps `display_offset` internally. After resize, `scrollbar_fade.mark_scrolled(now)` so the user sees the adjusted thumb briefly.
- **Tab restart (Ctrl+Shift+R) while scrolled up.** `restart()` rebuilds Self; new `scrollbar_fade = ScrollbarFade::new(default_scrollbar_fade_ms)`; `display_offset = 0`. The Stage 11 fields propagation pattern in `App::restart_active` also propagates `scrollbar_fade.fade_ms`.
- **`history_lines = 0` config**. Force minimum 1 in `apply_scrollback` (so alacritty_terminal's grid construction doesn't panic). Document in config comment: "0 effectively means no scrollback; use 1 for the same effect."
- **Mouse-wheel scroll while context menu is open** (Stage 10). Wheel events route to scrollback; menu stays open at its anchor. The menu's input intercept handles Up/Down/Enter/Esc only; wheel passes through. Verify this doesn't break menu's `CursorMoved` hover handling (which IS routed through menu when open).
- **Wheel in tab strip area** (cursor above `bar_h`). Always routes to active tab. The decision "wheel scrolls focused tab regardless of cursor position" matches Stage 10's right-click-anywhere convention.
- **`display_offset == history_size`** (top of history). `Term::scroll_display(Scroll::Delta(n))` clamps internally per alacritty's behavior. No infinite scroll up.
- **`display_offset > 0` and user presses Esc or a bare modifier.** No snap-to-bottom (key_to_bytes returns None). User can scroll back manually.
- **Selection across scrollback + live transition.** Selection-text-extraction was correct from Stage 10. Stage 12 fixes the rect-render filter so the on-screen portion of the selection is visible. Off-screen portions still don't render rects (correct).
- **`Term::mode()` mid-flight changes** (TUI app toggles mouse-reporting on/off). Each wheel event checks fresh; transition is seamless.
- **Multiple tabs each with their own scroll position**. Per-`PtySession` state (scrollbar_fade is per-session; `display_offset` lives in the per-session `Term`). Switching tabs doesn't reset scroll position; each tab remembers where you were.
- **Render scheduling while fading.** Renderer checks `fade_alpha > 0` after rendering; if true, schedules another redraw via `window.request_redraw()` so the fade-out animates. When `fade_alpha == 0`, no redraw scheduled — vibeflow returns to event-driven mode.

## Testing strategy

### Unit tests (render::scrollbar)

```rust
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
    assert!((half - 0.5).abs() < 0.05);
    let done = f.alpha(now + Duration::from_millis(1100));
    assert_eq!(done, 0.0);
}

#[test]
fn build_scrollbar_rects_empty_when_alpha_zero() {
    let rects = build_scrollbar_rects(0.0, 100, 5000, 24, (800.0, 600.0), 40.0, ScrollbarColors::default());
    assert!(rects.is_empty());
}

#[test]
fn build_scrollbar_rects_empty_when_at_bottom_no_history() {
    let rects = build_scrollbar_rects(1.0, 0, 0, 24, (800.0, 600.0), 40.0, ScrollbarColors::default());
    assert!(rects.is_empty());
}

#[test]
fn build_scrollbar_rects_thumb_at_bottom_when_display_offset_zero() {
    let rects = build_scrollbar_rects(1.0, 0, 5000, 24, (800.0, 600.0), 40.0, ScrollbarColors::default());
    assert_eq!(rects.len(), 2);
    // Track at top of available area, thumb at bottom of track.
    let track = rects[0];  // bbox (x, y, w, h, color)
    let thumb = rects[1];
    let track_bottom = track.1 + track.3;
    let thumb_bottom = thumb.1 + thumb.3;
    assert!((thumb_bottom - track_bottom).abs() < 1.0);
}

#[test]
fn build_scrollbar_rects_thumb_min_height_clamps() {
    // Very large history with small viewport.
    let rects = build_scrollbar_rects(1.0, 5000, 10000, 24, (800.0, 600.0), 40.0, ScrollbarColors::default());
    let thumb_h = rects[1].3;
    assert!(thumb_h >= 20.0, "thumb should be >= MIN_HEIGHT_PX (20)");
}
```

### Unit tests (render::selection)

```rust
#[test]
fn selection_rects_include_visible_scrollback_when_display_offset_nonzero() {
    // Construct a Term with display_offset = 10, history_size = 100.
    // Create a selection from line -5 (scrollback) to line 5 (live viewport).
    // With display_offset = 10, the line range from -5 to 5 is partially on-screen.
    // Assert rects include lines that are NOW on-screen.
}

#[test]
fn selection_rects_filter_off_screen_scrollback_when_display_offset_zero() {
    // Existing Stage 10 behavior: selection covers scrollback but rects are empty
    // for scrollback rows. Confirm Stage 12 didn't regress this.
}
```

### Unit tests (session::session)

```rust
#[test]
fn scroll_by_negative_advances_display_offset() {
    let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
    // Need to seed scrollback content. Simplest: write many lines via the dispatcher.
    // ...feed bytes...
    let initial = s.display_offset();
    let now = Instant::now();
    s.scroll_by(-5, now);
    assert!(s.display_offset() > initial);
}

#[test]
fn scroll_by_zero_is_noop() {
    let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
    let before = s.scrollbar_fade.clone();
    s.scroll_by(0, Instant::now());
    // scrollbar_fade.last_scroll_at should NOT have updated.
    // (Use a helper accessor or just compare debug strings if Clone is in.)
}

#[test]
fn scroll_to_bottom_resets_display_offset() {
    let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).expect("spawn");
    // ...feed bytes to grow history...
    let now = Instant::now();
    s.scroll_by(-10, now);
    assert!(s.display_offset() > 0);
    s.scroll_to_bottom(now);
    assert_eq!(s.display_offset(), 0);
}
```

### Unit tests (window — with mocked PTY where possible)

```rust
#[test]
fn mouse_wheel_routes_to_scrollback_when_not_in_mouse_mode() {
    // Setup a WindowApp with a session that has mouse mode OFF.
    // Send a WindowEvent::MouseWheel.
    // Assert session.display_offset() changed; PTY did not receive bytes.
}

#[test]
fn mouse_wheel_routes_to_pty_when_in_mouse_mode() {
    // Setup a WindowApp with a session in mouse mode.
    // Send a WindowEvent::MouseWheel.
    // Assert PTY received mouse-encoded wheel bytes; display_offset unchanged.
}

#[test]
fn keystroke_snaps_to_bottom_when_scrolled_up() {
    // Scroll up, then press a character key.
    // Assert display_offset == 0.
}

#[test]
fn bare_modifier_press_does_not_snap_to_bottom() {
    // Scroll up, then press Ctrl alone (no character).
    // Assert display_offset unchanged.
}
```

### Integration tests (`crates/vibeflow/tests/scrollback.rs`)

```rust
#[test]
fn select_all_with_scrollback_includes_history() {
    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn bash");
    // Drive the session to produce 200 lines of output.
    // Run `seq 1 200` via send_input.
    // Tick until output is processed.
    // Call select_all_active; copy via selection.text().
    // Assert text contains lines like "1\n", "2\n", ..., "200\n".
}

#[test]
fn scroll_by_then_scroll_to_bottom_round_trip() {
    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn");
    // Seed enough output to have history.
    let active = app.active();
    let now = Instant::now();
    app.tabs_mut()[active].scroll_by(-10, now);
    assert!(app.tabs()[active].display_offset() > 0);
    app.tabs_mut()[active].scroll_to_bottom(now);
    assert_eq!(app.tabs()[active].display_offset(), 0);
}
```

### Manual smoke walk on VNC

1. Open vibeflow, run `seq 1 200`. Wheel up — viewport scrolls into history; scrollbar fades in on the right.
2. Wheel back down. Scrollbar fades out after ~1.5 s.
3. Touchpad two-finger scroll — same behavior (winit synthesizes wheel).
4. Shift+PageUp — half-page jump up. Shift+PageDown — back down. Ctrl+Home — top of history. Ctrl+End — back to live.
5. While scrolled up, run `seq 1 50` (briefly tab to bottom, run, scroll back up before output finishes). New output stacks invisibly. Scroll position holds.
6. While scrolled up, press a character key. Snaps to bottom. Character appears at prompt.
7. While scrolled up, press Esc or Ctrl alone. NO snap (no bytes produced).
8. Open `vim`, enable mouse mode if needed (most vims do automatically in alt-screen). Wheel — vim scrolls. Exit vim — vibeflow scrolling restored.
9. Open `less` on a big file (e.g., `journalctl | less`). Wheel — less scrolls. q to exit.
10. Select text spanning live + scrollback rows. Copy. Paste elsewhere — full selected text present.
11. Right-click on grid while scrolled up — menu opens (Stage 10). Wheel — scrollback scrolls; menu stays open.
12. Edit config: `[scrollback] history_lines = 100`. Save, then spawn a new tab. Run `seq 1 200`. Only last 100 lines retained.
13. Edit config: `[scrollback] scrollbar_fade_ms = 500`. Save (no restart). Scroll — scrollbar fades out in ~500 ms instead of 1500.
14. Resize window while scrolled up — viewport reflows; thumb adjusts; no crash.
15. Open multiple tabs, scroll each to different positions, switch between them — each tab remembers its own scroll position.

## Implementation sequencing (rough — refined in plan)

1. `render::scrollbar::ScrollbarFade` + `build_scrollbar_rects` + `ScrollbarColors` + unit tests.
2. `[scrollback]` config schema + resolved struct + defaults + apply step + tests.
3. Two new `[colors]` keys + Renderer setter + apply wiring.
4. `PtySession.scrollbar_fade` field + initialization in `spawn` + `scroll_by` / `scroll_to_top` / `scroll_to_bottom` / `display_offset` methods + tests.
5. `App::set_default_scrollbar_fade_ms` setter; `App::new_tab` and `App::restart_active` propagate.
6. `Renderer::render` cell-pass walks rows with `display_offset` (read-through to alacritty).
7. `Renderer::render` appends scrollbar rects + schedules redraw while fade > 0.
8. `build_selection_rects` lifts scrollback filter when on-screen.
9. `WindowApp::MouseWheel` handler with mouse-mode gating.
10. `WindowApp::KeyboardInput` chord handlers (Shift+PgUp/Dn, Ctrl+Home/End).
11. Snap-to-bottom on `key_to_bytes`-producing keystrokes.
12. `apply_config` wires `[scrollback]` + `[colors] scrollbar_*` into App + sessions.
13. Integration tests against real PTY.
14. Senior pre-execution Sonnet review.
15. Manual smoke walk on VNC.
16. Senior holistic Sonnet review at end of stage.

## Risks & mitigations

- **`Scroll::Delta` sign convention.** Plan-verbatim code assumes positive = forward (toward live). Easy to invert. Mitigation: senior pre-execution review verifies against the crate source; unit test asserts the direction.
- **`mouse_encoder` lacks wheel-button variants.** Confirmed by spec self-review against `render/mouse_encoder.rs:26-37`: `Button` enum is `{Left, Middle, Right}` only, and `encode_press` maps these to codes 0/1/2. Wheel is xterm button codes 4 (up) and 5 (down). Stage 12 extends `mouse_encoder` with `Button::{WheelUp, WheelDown}` and updates `encode_press`'s match (or adds an `encode_wheel` helper). This is a NEW code change in Stage 12, not just a call to existing API. Plan must include it as an explicit task.
- **Selection-rect rendering with display_offset > 0.** The line-index → on-screen y translation has off-by-ones if the line origin is treated as the wrong reference. Mitigation: explicit unit tests for known display_offset values.
- **Scrollbar interaction with context menu.** Wheel during menu-open needs to NOT dismiss the menu. Mitigation: the menu input intercept (Stage 10) handles a specific set of events (Up/Down/Enter/Esc/click); wheel falls through to scrollback. Test in manual smoke.
- **Render scheduling for fade animation.** Vibeflow only redraws on dirty events; fading requires continuous redraws while fade is active. Mitigation: explicit `request_redraw` call in `Renderer::render` when `fade_alpha > 0`. Once fade hits 0, no more redraws scheduled (no busy-redraw at rest).
- **`history_lines` not retroactive.** alacritty_terminal grid is sized at construction; mid-life history resize isn't supported. Mitigation: document in config comment; setting applies to new tabs.
- **TUI app toggling mouse-mode mid-session.** Each wheel event checks fresh `term.mode()`. Transitions are seamless.

## Out-of-scope notes for future stages

- **Stage 13 polish bundle**: indicator visual prominence (user-prioritized), Shift/Ctrl arrow keys for word/line jump (Stage 8 deferral), block selection, bell behavior config, font priority live-reload, iTerm2 color-scheme import.
- **Search-in-scrollback** (`/`-style search): explicitly out of v0.1.
- **Splits/panes**: explicitly out of v0.1.
- **Alternative scroll wheel acceleration curves**: cheap polish item, defer to Stage 13 if needed.
- **Persistent scroll position across config reload**: not in scope — config reload shouldn't reset scroll, but if it does we'll address in Stage 13.
