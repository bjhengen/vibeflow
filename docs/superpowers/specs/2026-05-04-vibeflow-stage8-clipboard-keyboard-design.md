# vibeflow Stage 8 Design: Clipboard + Keyboard Shortcuts + Selection

**Date:** 2026-05-04
**Status:** approved (brainstorm); pending implementation plan
**Predecessor:** Stage 7.5 (`stage7.5-color-emoji-complete`, merged in `7b2e2b1`).
**Successor:** Stage 9 (TOML config + hot-reload + scrollback rendering).

## Summary

Stage 7.5 closed out the rendering pipeline: color emoji, wide glyphs, and the
fundamental dual-atlas + write-once-per-pipeline machinery. What's missing for
vibeflow to be a usable daily-driver terminal is *interaction* — keyboard
shortcuts that match every other Linux terminal's muscle memory, mouse-driven
text selection, copy/paste that survives multi-line shell snippets, and a way
to recover a session that died via `Ctrl+D`.

Stage 8 adds three layered subsystems plus a thin dispatch layer:

- **`SelectionTracker`** — pure-logic state machine that consumes mouse events
  and produces a per-cell selection. Lives on `PtySession` so each tab owns its
  own selection state across tab switches.
- **`Clipboard`** — thin `arboard`-backed wrapper over the system CLIPBOARD
  selector. PRIMARY (X11 select-to-copy / middle-click-paste) is deferred.
- **`keymap`** — single source of truth for the shortcut dispatch table:
  Ctrl+Shift+{T,W,R,C,V,Tab} plus Super-modifier aliases for Mac-via-VNC.

The dispatch layer in `window.rs` interrogates `alacritty_terminal`'s mouse-mode
flag to route mouse events between the selection tracker and the PTY (for `vim`,
`tmux`, `htop` mouse support), with `Shift` as the per-event override.

## Goals

- Keyboard: `Ctrl+Shift+{T,W,Tab,Shift+Tab,R,C,V}` work as in gnome-terminal,
  plus `Super+{T,W,Tab,Shift+Tab,R,C,V}` aliases for Mac muscle memory over
  VNC. `Ctrl+C` continues to send SIGINT to the PTY; `Ctrl+V` continues to
  send `quoted-insert` (0x16).
- Mouse: drag-select (linear text-flow), double-click word, triple-click line.
  Selection renders as a 40%-alpha highlight overlay on cell glyphs. Cell-aligned.
- Mouse passthrough: when the running app enables terminal mouse mode
  (`vim` `set mouse=a`, `htop`, `tmux`), mouse events go to the PTY as SGR
  escape sequences. `Shift+drag` overrides and selects anyway.
- Clipboard: `Ctrl+Shift+C` copies the active session's selection to the
  system CLIPBOARD; `Ctrl+Shift+V` pastes. Bracketed paste markers are added
  automatically when the term has `\x1b[?2004h` mode active.
- Restart: `Ctrl+Shift+R` on a *dead* tab kills the residual handles and
  re-spawns `$SHELL` (fallback `bash`) with the same TTY size and the user's
  current working directory. No-op on live tabs.
- Selection persists across tab switches (per-tab state). Clears on input,
  resize, single-click without drag, or tab close.

## Non-Goals (deferred)

- **PRIMARY clipboard / middle-click paste** — Linux X11 power-user feature.
  Stage 9 or beyond, as a TOML config flag (off by default to keep Mac/Wayland
  behavior consistent). Brian's primary workflow is Mac-over-VNC; PRIMARY adds
  no value there.
- **Right-click context menu** — needs an overlay-rendering subsystem (floating
  box positioning, per-item hit-test, dismiss-on-outside, keyboard nav).
  Substantial scope, deferred to Stage 9 or 10. Right-click stays available
  for terminal-mouse-mode passthrough into vim/htop/tmux.
- **Block (column) selection** — `Alt+drag` rectangular selection. Useful for
  tabular data (`top`, `htop`, `ps`) but separate state machine. Stage 9 or
  beyond. The `SelectionTracker` API is shaped so it can be added without
  breaking existing callers — `SelectionMode::Block` slots into the `mode`
  enum alongside `Cell`/`Word`/`Line`.
- **Configurable shortcuts** — the `keymap` table is hard-coded. Stage 9 will
  read it from TOML. Centralizing dispatch in one file now makes that diff
  small.
- **Configurable selection color** — also hard-coded; Stage 9 reads from TOML.
- **Selection that anchors to grid content (survives scroll)** — Stage 8
  selection is anchored to grid coordinates only. If a background tab scrolls
  while a selection is held, the highlighted cells now show different content;
  this matches alacritty/wezterm/kitty behavior. Stage 9 or beyond if a
  follow-on use case demands it.
- **Selection in scrollback** — Stage 8 can only select cells in the visible
  display region. Scrollback rendering itself is Stage 10+; selection in
  scrollback comes with that.
- **Search / find-in-buffer** — entirely separate subsystem, Stage 10+.

## Architecture

**Pattern: per-session selection state, system-clipboard singleton, keymap
table, mouse-mode-aware dispatch.**

```
                                  +-------------------+
                                  |  winit events     |
                                  +---------+---------+
                                            |
                                  +---------v---------+
                                  |   window.rs       |
                                  |   dispatch        |
                                  +---+--+--+---+-----+
                                      |  |  |   |
                       +--------------+  |  |   +-----------------+
                       |                 |  +---+                 |
                       v                 v      v                 v
               +-------+----+    +-------+--+ +--+--------+  +----+----+
               | keymap     |    | Selection| | term      |  | tab bar |
               | table      |    | Tracker  | | encoder   |  | hit     |
               | (shortcut) |    | (per-PtS)| | (SGR mouse|  | (Stage 6|
               +-----+------+    +-----+----+ |  passthrough)|  + new)  |
                     |                 |     +-----+-----+   +---------+
                     v                 v           v
            +--------+--------+   +----+-----+ +---+-----+
            | App actions:    |   | Selection| | PTY     |
            | new/close/cycle |   | rect     | | bytes   |
            | restart, copy,  |   | render   | +---------+
            | paste           |   +----------+
            +-----------------+
```

Four new modules, five files modified.

### `crates/vibeflow/src/render/selection.rs` (NEW, ~150 LOC)

Pure-logic state machine. No GPU, no winit, no PTY dependency.

```rust
pub struct SelectionTracker {
    selection: Option<Selection>,
    drag_anchor: Option<Point>,
    click: ClickHistory,
}

pub struct Selection {
    pub start: Point,
    pub end: Point,
    pub mode: SelectionMode,
}

pub enum SelectionMode { Cell, Word, Line }

struct ClickHistory {
    last_at: Instant,
    last_point: Point,
    count: u8,
}

impl SelectionTracker {
    pub fn mouse_down(&mut self, point: Point, shift_held: bool, term: &Term);
    pub fn mouse_drag(&mut self, point: Point, term: &Term);
    pub fn mouse_up(&mut self);
    pub fn clear(&mut self);
    pub fn is_dragging(&self) -> bool;
    pub fn current(&self) -> Option<&Selection>;
    pub fn cells(&self) -> impl Iterator<Item = Point> + '_;
    pub fn text(&self, term: &Term) -> Option<String>;
}
```

`Point` is `alacritty_terminal::index::Point`. Word / line snapping uses the
existing `term.semantic_search_left` / `_right` helpers (default word chars are
alphanumeric + underscore).

Click counter resets when:
- More than 500 ms have elapsed since the last click, OR
- The new click point is more than 1 cell away from the last click point.

`mouse_up` checks if `start == end` AND `click.count == 1` (a click without a
drag) and calls `clear()` in that case — single-click clears prior selection.

### `crates/vibeflow/src/clipboard.rs` (NEW, ~60 LOC)

```rust
pub struct Clipboard { inner: arboard::Clipboard }

impl Clipboard {
    pub fn new() -> Result<Self>;
    pub fn copy(&mut self, text: &str) -> Result<()>;
    pub fn paste(&mut self) -> Option<String>;
}
```

CLIPBOARD-only; no PRIMARY. Errors are logged at `warn` and treated as no-ops —
a clipboard-server hiccup must not crash the renderer.

### `crates/vibeflow/src/keymap.rs` (NEW, ~80 LOC)

```rust
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    RestartTab,
    Copy,
    Paste,
}

pub fn match_shortcut(
    key: &winit::keyboard::Key,
    modifiers: winit::keyboard::ModifiersState,
) -> Option<Shortcut>;
```

The dispatch table is the only file that needs changing in Stage 9 to support
configurable shortcuts. Modifier matching rule: a combo matches if
`Ctrl+Shift+Key` *or* `Super+Key` (without Shift, except `Ctrl+Shift+Tab` /
`Super+Shift+Tab` where Shift is the directional sign). `Alt` and other
modifiers must be unset to avoid false positives.

### `crates/vibeflow/src/render/mouse_encoder.rs` (NEW, ~80 LOC)

Encodes a mouse event (`button`, `point`, `pressed`/`released`/`drag`) into the
byte sequence the running app expects. SGR format
(`\x1b[<{button};{col};{row}{M|m}`) is used when the term has `SGR_MOUSE` mode
set; X10 (`\x1b[M{btn+32}{col+32}{row+32}`) is the fallback. SGR is what every
modern app uses; X10 is here for historical completeness.

### Modifications

- **`session/session.rs`** — `PtySession` gains a `selection: SelectionTracker`
  field and a `restart(&mut self) -> Result<()>` method. `restart`:
  1. `child.kill()` (no-op if already dead).
  2. Drop the old reader-thread mpsc receiver; new spawn gets a fresh channel.
  3. Read TTY size via `master.get_size()`.
  4. `PtySession::spawn(&[$SHELL or "bash"], cwd, size)?`.
  5. `*self = new_session` — replace internals in place.
- **`app.rs`** — `restart_active(&mut self)` and `cycle_active(direction: i32)`.
  Existing `new_tab` and `close_tab` already cover the corresponding shortcuts.
- **`window.rs`** —
  - `KeyboardInput` arm: call `keymap::match_shortcut` *before* the existing
    typed-input fallthrough. If `Some(shortcut)`, dispatch; else fall through
    so the literal keystroke reaches the PTY.
  - `MouseInput` and `CursorMoved` arms: branch on
    `term.mode().intersects(MOUSE_REPORT_CLICK | MOUSE_DRAG | MOUSE_MOTION)`
    AND `!modifiers.shift_key()`:
    - True → encode via `mouse_encoder` and write to PTY.
    - False → dispatch to active session's `SelectionTracker`.
  - Right and middle button: pass-through to PTY only when mouse mode is on.
    Otherwise no-op.
- **`render/mod.rs`** — pull `app.tabs[active].selection.cells()`, convert each
  to a `RectInstance` with `SELECTION_COLOR` (= `[0.4, 0.6, 1.0, 0.4]`),
  append into the existing `all_rects` Vec between tab rects and banner rect.
  Add one new `tab_bar_pipeline.draw_range(...)` call between tab text and
  banner rect.

**Net delta:** +500 / -30 across 9 files; one new dependency (`arboard`). The
`mouse_encoder.rs` was an addition during the design pass — the brainstorm
called for three new files but the SGR/X10 encoding is genuinely separate
from selection state (it's a pure PTY-passthrough concern), so factoring it
into its own ~80 LOC module is cleaner than burying it inside `window.rs`.

## Mouse-event flow in detail

```
WindowEvent::MouseInput { state: Pressed, button: Left, .. }
└─ if cursor_pos.y < layout.bar_height_px:
│      → existing tab bar hit-test (Stage 6)
└─ else if Some(point) = pixels_to_grid(cursor_pos):
       let mode_on = term.mode().intersects(MOUSE_REPORT_CLICK
                                             | MOUSE_DRAG
                                             | MOUSE_MOTION);
       let shift_override = modifiers.shift_key();
       if mode_on && !shift_override:
           bytes = mouse_encoder::encode_press(point, button, mode_flags);
           session.send_input(&bytes);
       else:
           session.selection.mouse_down(point, shift_override, term);

WindowEvent::CursorMoved (during drag)
└─ if cursor_pos.y < bar_height_px:
│      → existing tab-bar hover (no-op for now; Stage 6 left this open)
└─ else if Some(point) = pixels_to_grid(...):
       if mode_on && motion-tracking-enabled && !shift_held:
           bytes = mouse_encoder::encode_drag(point, mode_flags);
           session.send_input(&bytes);
       elif session.selection.is_dragging():
           session.selection.mouse_drag(point, term);

WindowEvent::MouseInput { state: Released, .. }
└─ symmetric — encode SGR release, or call mouse_up().

Right/Middle button:
└─ if mode_on: encode and pass to PTY. Else: no-op.
```

## Selection rendering layering

Post-Stage-7.5, rect instances live in a single per-frame buffer with this
layout:

```
[tab rects][selection rects][banner rect (opt)][bell rect (opt)]
```

Draw order in `Renderer::render`:

```
1. quad_pipeline.draw_range(0..cell_count)                — cells
2. tab_bar_pipeline.draw_range(0..tab_rect_count)         — tab rects
3. quad_pipeline.draw_range(tab_glyph_offset..banner_glyph_offset)
                                                          — tab text
4. tab_bar_pipeline.draw_range(selection_offset..banner_rect_offset)
                                                          — selection ← NEW
5. tab_bar_pipeline.draw_range(banner_rect_offset..bell_rect_offset)
                                                          — banner rect (dead)
6. quad_pipeline.draw_range(banner_glyph_offset..total_quads)
                                                          — banner glyphs
7. tab_bar_pipeline.draw_range(bell_rect_offset..total_rects)
                                                          — bell flash
```

Selection draws after cells (so the highlight is visible on top of glyphs at
40% alpha), and before the banner (so a dead-tab banner correctly dims
selection-on-cells underneath it). Tab rects don't overlap selection
coordinates, so their relative draw order is moot.

Selection cell to rect conversion:

```rust
const SELECTION_COLOR: [f32; 4] = [0.4, 0.6, 1.0, 0.4]; // light blue, 40% alpha

for point in selection.cells() {
    let screen_x = (point.column.0 as u32 * cell_w) as f32;
    let screen_y = (point.line.0 as u32 * cell_h + bar_height_px) as f32;
    rects.push(RectInstance::new(
        screen_x, screen_y, cell_w as f32, cell_h as f32, SELECTION_COLOR,
    ));
}
```

`tab_bar_pipeline` already handles solid-color rects with alpha and is what the
banner / bell already use.

## Keyboard shortcuts table

| Shortcut | Action | Notes |
|---|---|---|
| `Ctrl+Shift+T` or `Super+T` | New tab | Spawn `$SHELL`, append, set active |
| `Ctrl+Shift+W` or `Super+W` | Close active tab | Last-tab close → window stays open with no tabs; `Ctrl+Shift+T` recovers |
| `Ctrl+Tab` or `Super+Tab` | Next tab | Wraps; cycles dead tabs too |
| `Ctrl+Shift+Tab` or `Super+Shift+Tab` | Previous tab | Wraps |
| `Ctrl+Shift+R` or `Super+R` | Restart dead session | No-op on live tabs (logged at `trace`) |
| `Ctrl+Shift+C` or `Super+C` | Copy selection | If no selection, no-op |
| `Ctrl+Shift+V` or `Super+V` | Paste | Bracketed-paste-wrapped if term reports `\x1b[?2004h` |

`Ctrl+C` continues to send SIGINT (0x03 byte) and `Ctrl+V` continues to send
`quoted-insert` (0x16). The shortcut layer sits *before* the typed-input
fallthrough in `window.rs`, so a matched shortcut suppresses the literal byte;
an unmatched key falls through to `send_input(bytes)` as before.

WM-grab edge case: some Linux WMs grab `Super+Tab` for window switching. If the
WM grabs it, vibeflow never sees the keystroke (silent passthrough — not a bug,
just a WM behavior). `Ctrl+Tab` always works as the primary binding.

## Bracketed paste

`alacritty_terminal` already tracks the `BRACKETED_PASTE` flag from the
`\x1b[?2004h` enable / `\x1b[?2004l` disable codes. bash/zsh/fish enable it
during their prompt; vim disables it in insert mode. We just check the flag at
paste time:

```rust
let Some(text) = self.clipboard.paste() else { return; };
let session = match self.app.tabs_mut().get_mut(self.app.active()) {
    Some(s) => s, None => return
};
let bracketed = session.term().mode().contains(TermMode::BRACKETED_PASTE);
if bracketed {
    session.send_input(b"\x1b[200~");
    session.send_input(text.as_bytes());
    session.send_input(b"\x1b[201~");
} else {
    session.send_input(text.as_bytes());
}
```

## Restart semantics

Closing a session with `Ctrl+D` writes "exit\r\n" to the terminal grid (bash's
default), the child exits, the reader thread terminates, and `Died` is emitted.
The tab stays in `App::tabs` with `is_alive() == false`. The dead-tab banner
("session died -- press Ctrl+Shift+R to retry") draws over the cell grid.

`Ctrl+Shift+R` on the dead tab calls `PtySession::restart`:

```rust
pub fn restart(&mut self) -> Result<()> {
    let _ = self.child.kill();
    drop(std::mem::replace(&mut self.rx, mpsc::channel().1));

    let size = self.master.get_size()?;
    let argv = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
    let cwd = std::env::current_dir().ok();

    let new_session = PtySession::spawn(&[&argv], cwd.as_deref(), size)?;

    *self = new_session;
    Ok(())
}
```

The new session takes over the same tab slot. Selection state drops with the
old `SelectionTracker`. Label resets to the new shell's default.

`App::restart_active`:

```rust
pub fn restart_active(&mut self) -> Result<()> {
    let Some(s) = self.tabs.get_mut(self.active) else { return Ok(()) };
    if s.is_alive() {
        log::trace!("Ctrl+Shift+R on live tab; ignoring");
        return Ok(());
    }
    s.restart()
}
```

The `is_alive()` guard means accidentally pressing `Ctrl+Shift+R` on a live tab
is a no-op rather than a destructive kill-and-respawn.

## Selection lifecycle

Selection is *finalized* (visible, ready for copy) on `mouse_up` after a drag.
It *clears* on:

- Any keystroke that produces input bytes (typing, Enter, arrow keys, escape
  sequences from the typed-input path).
- Window resize (cell coordinates may invalidate; clear is safer than reflow).
- Single-click without drag (`mouse_up` with `start == end` and `count == 1`).
- Tab close (the whole `SelectionTracker` drops with the `PtySession`).

Selection *persists* across:

- Tab switch — each `PtySession` owns its own `SelectionTracker`. Switching
  tabs doesn't touch any session's state. Switching back finds the selection
  intact.
- Focus changes — vibeflow doesn't track focus state separately.
- Scroll, mouse-mode toggles, terminal output writes — selection coordinates
  stay; the cells they refer to may now hold different content. Accepted
  behavior for v0.1, matching alacritty/kitty/wezterm.
- Successful copy via `Ctrl+Shift+C` — selection stays visible after copy
  (matches gnome-terminal). User can copy multiple times or modify the
  selection before clearing.

## Test counts

Before Stage 8: 135 default + 12 ignored.
After Stage 8: ~170-175 default + ~15 ignored.

Breakdown:
- `SelectionTracker` state machine: ~20 default tests.
- `keymap::match_shortcut` table: ~10 default tests.
- `mouse_encoder` SGR encoding: ~6 default tests (round-trip click/release/drag
  with known-good byte sequences; X10 fallback is one of them).
- `PtySession::restart` integration: ~2 tests in the existing PTY integration
  module (restart-after-Ctrl+D and restart-while-still-alive).
- Selection rendering (`build_selection_rects`): ~3 ignored (need `LIBGL`
  software-GL).
- Bracketed-paste wrap is verified via manual smoke (multi-line paste into
  bash should not auto-execute) — the wrap itself is a 4-line if/else in
  `window.rs`, not worth its own test scaffold.

Workspace `cargo fmt`, `clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc` all clean.
60s libfuzz on `vibeflow-protocol`: no regression.

## Risks and mitigations

1. **`arboard` clipboard reliability over VNC** — clipboard sync between Mac
   client → VNC server → Linux X11 has historically been spotty. arboard's
   X11 backend uses an event loop that competes with winit's. If sync drops,
   `clipboard.copy()` returns `Ok(())` but the Mac side never sees the text.
   *Mitigation:* the smoke checklist explicitly tests "copy in vibeflow on
   host → paste into a Mac app". If it's flaky, Stage 9 can add a small
   X11 selection-handler thread.
2. **WM Super-key grabs** — `Super+T` may be grabbed by GNOME / KDE / i3 / sway
   and never reach vibeflow. *Mitigation:* `Ctrl+Shift+T` is the primary
   binding. Smoke documents Super-grab behavior. Stage 9 TOML config can
   disable Super aliases entirely if needed.
3. **`Shaping::Advanced` shaping cost on fast typing** — Stage 7.5's
   `Shaping::Advanced` adds rustybuzz cost per glyph cache miss. At normal
   typing speed this is invisible (cache fills quickly), but pasting a 10K-line
   blob via `Ctrl+Shift+V` could hit a cluster of misses. *Mitigation:* the
   bracketed-paste wrap is tiny; the actual rasterization cost is bounded by
   the number of distinct codepoints, not by paste size. Smoke tests a
   multi-line paste of an `.rs` file.
4. **SelectionTracker scrollback awareness** — Stage 8 selection is anchored to
   absolute grid line numbers. When the buffer scrolls, the highlighted cells
   change content. *Mitigation:* documented as known v0.1 behavior; matches
   every other terminal.
5. **Mouse-mode toggle mid-drag** — if vim toggles mouse mode while the user is
   mid-drag, the routing decision flips. *Mitigation:* the `is_dragging()`
   flag means once a selection drag has started, drag/up events stay with the
   tracker until release, regardless of mid-drag mode flips.
6. **PTY-restart leaving the reader thread alive** — `child.kill()` is async on
   Unix. The reader thread will see EOF when the PTY half-closes, but there's
   a brief window where it's still alive. *Mitigation:* `drop(replace(rx, ...))`
   drops the receiver; the reader's `send` then returns `Err` and the thread
   exits cleanly. Test exercises this.

## Open questions resolved during brainstorm

| Question | Resolution |
|---|---|
| Ctrl+C/V vs Ctrl+Shift+C/V | Ctrl+Shift+C/V (matches gnome-terminal; Ctrl+C must remain SIGINT) |
| Cmd alias scope | All shortcuts get Super alias (full Mac-via-VNC parity) |
| Restart re-spawn target | `$SHELL` (fallback `bash`); argv-replay deferred to Stage 9 |
| Selection style | Linear text-flow only; block-selection deferred |
| Bracketed paste | Always on when term reports `\x1b[?2004h` |
| PRIMARY clipboard | Skipped; revisit if requested |
| Right-click | No action in Stage 8; menu is Stage 9 / 10 territory |
| Mouse mode passthrough | Standard pattern (mouse-mode-on → PTY; Shift override → tracker) |
| Double / triple click | Both included (word / line) |
| Selection persistence | Per-tab, persists across tab switches |
