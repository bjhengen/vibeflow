# vibeflow — Stage 10: Overlay UX (right-click menus + blink-synced rename caret)

**Status:** Draft, pending review
**Date:** 2026-05-09
**Author:** brainstormed with Claude

## Summary

Stage 10 adds an overlay UX layer on top of Stage 9's configuration foundation: right-click context menus on tabs and the grid, full keyboard navigation within those menus, and a phase-locked blinking caret on the inline rename overlay. The implementation is **tactical, not generic** — there's no new "OverlayLayer" abstraction; instead, a new `ContextMenuState` field on `WindowApp` slots in alongside the existing `RenameInputState` and `BellFlash` patterns established in Stages 8–9.

Scrollback rendering, in-buffer search, command palette, and tooltips remain out of scope. They remain candidates for later stages and v0.2.

## Goals & Non-Goals

### Goals

- **Right-click on a tab** opens a tab-context menu with `Rename Tab`, `Restart Tab` (only when the tab is dead), `New Tab`, `Close Tab`, `Close Other Tabs`.
- **Right-click on the grid** opens a grid-context menu with `Copy` (disabled when no selection), `Paste`, `Paste Selection`, `Select All`, `Clear Buffer`, `Open Config…`, `About vibeflow`.
- Both menus are fully keyboard-navigable: Up / Down / Enter / Esc, plus mouse hover and click-outside-to-dismiss.
- The inline rename caret introduced in Stage 9 blinks in phase with the active tab's terminal cursor by sharing the existing `CursorBlink` instance. When `[cursor] blink_ms = 0`, both render solid.
- One new `Shortcut` enum variant: `SelectAll` (Ctrl+Shift+A by default). Other new menu actions are menu-only and do not get keybindings.
- New `[colors]` keys for menu styling: `menu_bg`, `menu_border`, `menu_text`, `menu_text_disabled`, `menu_shortcut`, `menu_focus_bg`. Sane dark-theme defaults.

### Non-Goals (Stage 10)

- A generalized `Overlay` trait or stack. (Tactical implementation only — see *Architecture: scope choice*.)
- Scrollback rendering or selection in scrollback. (Deferred to a later stage; explicit dependency on this overlay subsystem is none.)
- Tooltips, hover help, or any second overlay type beyond the context menu. ("About vibeflow" opens the GitHub URL via `xdg-open` rather than a dedicated info panel — see *Action specifics*.)
- New keyboard shortcuts beyond `SelectAll`. Menu-only actions stay menu-only this stage.
- Animated open/close (fade or scale). The menu appears and disappears instantly.
- Sub-menus / cascading menus. All items are flat.
- Per-platform menu integration (e.g., GTK menu styling on Linux). Vibeflow renders its own menu via existing `wgpu` pipelines.

## Architecture

### Scope choice: tactical, not generic

vibeflow already has overlay-shaped features wired tactically: `BellFlash` is a field on `Renderer`, and `RenameInputState` is a field on `WindowApp` that the tab strip renderer consumes. There is no shared trait, no `OverlayLayer`, no z-stack — each is a one-off field that the render pass and input router check explicitly.

Stage 10 follows that same tactical pattern. `ContextMenuState` becomes another `Option` field on `WindowApp` and the render pass simply renders it last. Reasoning:

- **Single immediate consumer.** Only the context menu needs the abstraction; no other v0.1 roadmap item would consume it. (Search bar is explicitly out of scope; tooltips are deferred.)
- **YAGNI.** Building a trait + dyn + lifetime soup for one consumer is speculative. If/when a second overlay-style feature lands, the existing tactical impls have clean enough boundaries that retrofitting a shared layer is straightforward.
- **Rust-newcomer-friendly.** Three concrete `Option<…State>` fields are easier to reason about than `Vec<Box<dyn Overlay>>` plus the trait's lifetime obligations.

### Module layout

| File | Status | Purpose |
|---|---|---|
| `crates/vibeflow/src/render/context_menu.rs` | NEW | Owns `ContextMenuState`, `MenuItem`, `MenuAction`, `MenuLayout`, anchoring math, focus-index handling, render call, and pure-logic unit tests. |
| `crates/vibeflow/src/render/mod.rs` | TOUCHED | Calls `context_menu::render(...)` last in the render pass (after the bell-flash overlay). Threads `&CursorBlink` into rename overlay rendering. |
| `crates/vibeflow/src/render/tabs.rs` | TOUCHED | `RenameInputState`'s render path gains a `cursor_blink: &CursorBlink` parameter; uses `cursor_blink.visible(now)` to gate the caret rect. |
| `crates/vibeflow/src/window.rs` | TOUCHED | Adds `context_menu: Option<ContextMenuState>` to `WindowApp`. Right-click handler routes to grid- or tab-menu construction. Input dispatch peeks at the menu first. Action dispatch translates `MenuAction` to existing or new handlers. |
| `crates/vibeflow/src/keymap.rs` | TOUCHED | Adds `Shortcut::SelectAll` enum variant and default binding (Ctrl+Shift+A). |
| `crates/vibeflow/src/config/schema.rs` | TOUCHED | Adds six menu color keys to the `[colors]` section with defaults. |

No new wgpu pipeline. The menu uses the existing `TabBarPipeline` (solid rects for background, border, focus highlight, separators) and `QuadPipeline` + `TextEngine` (text quads for labels and shortcut hints).

### Render order per frame

```
clear → grid cells → cursor → tab strip → bell flash → context menu
```

Context menu draws last so it sits above everything else.

### Input flow

```
KeyboardInput / MouseInput / CursorMoved
  └─→ if WindowApp.context_menu.is_some():
        ├─ ArrowUp / ArrowDown → focus_prev / focus_next; redraw; consume
        ├─ Enter               → activate focused item; close; dispatch
        ├─ Escape              → close; consume
        ├─ left-click inside   → activate clicked item (or no-op if disabled/separator); close; dispatch
        ├─ left-click outside  → close; consume the click (do NOT pass through)
        ├─ right-click anywhere → close current; if hit_region is tab or grid, open new menu there
        ├─ CursorMoved over enabled item → focused = item_index; redraw
        └─ any other keypress  → close; fall through to normal handling
      else:
        └─ existing Stage 9 input routing (unchanged)
```

This is the only place input routing changes. All Stage 9 keymap, mouse, and clipboard paths are preserved when no menu is open.

### Action dispatch

```rust
pub enum MenuAction {
    Shortcut(Shortcut),  // reuses existing handlers
    PastePrimary,        // reads PRIMARY selector, writes to target PTY
    ClearBuffer,         // writes 0x0c to target PTY
    CloseOtherTabs,      // closes all sessions except target
    OpenConfig,          // xdg-open ~/.config/vibeflow/config.toml
    OpenRepoUrl,         // xdg-open https://github.com/bjhengen/vibeflow
}
```

`MenuAction::Shortcut(s)` reuses Stage 9's per-shortcut dispatch path. The other variants get small new handlers in `WindowApp`.

## Components

### `ContextMenuState`

```rust
pub struct ContextMenuState {
    /// Anchor in physical pixels (where the right-click happened).
    anchor: (f32, f32),
    items: Vec<MenuItem>,
    /// Index into `items` of the focused row. Always points at an enabled,
    /// non-separator item.
    focused: usize,
    /// Set when the menu was opened from a tab right-click; identifies which
    /// session this menu's actions target. None for grid menus → target = active tab.
    target_session: Option<SessionId>,
    /// Cached layout: bbox in physical px + per-item rects. Recomputed on
    /// open and on window resize; reused for hit-testing each frame.
    layout: MenuLayout,
}

pub enum ItemKind {
    Action,
    Separator,
}

pub struct MenuItem {
    label: &'static str,
    shortcut_hint: Option<&'static str>, // "Ctrl+Shift+C", "Mid-click", or None
    action: MenuAction,
    enabled: bool,                       // resolved at open time
    kind: ItemKind,
}
```

`SessionId` is the existing identifier vibeflow uses to refer to a `PtySession`. (No change to that type.)

### Menu builders

Two pure functions in `render::context_menu`:

```rust
pub fn tab_menu(session_id: SessionId, is_dead: bool, tab_count: usize) -> Vec<MenuItem>;
pub fn grid_menu(has_selection: bool) -> Vec<MenuItem>;
```

`tab_menu` order:
1. Rename Tab — Ctrl+Shift+E, `Shortcut(RenameTab)`
2. Restart Tab — Ctrl+Shift+R, `Shortcut(RestartTab)` *(only present when `is_dead`)*
3. *Separator*
4. New Tab — Ctrl+Shift+T, `Shortcut(NewTab)`
5. *Separator*
6. Close Tab — Ctrl+Shift+W, `Shortcut(CloseTab)`
7. Close Other Tabs — no hint, `CloseOtherTabs` *(disabled when `tab_count == 1`)*

`grid_menu` order:
1. Copy — Ctrl+Shift+C, `Shortcut(Copy)` *(disabled when `!has_selection`)*
2. Paste — Ctrl+Shift+V, `Shortcut(Paste)`
3. Paste Selection — "Mid-click" hint, `PastePrimary`
4. *Separator*
5. Select All — Ctrl+Shift+A, `Shortcut(SelectAll)`
6. Clear Buffer — no hint, `ClearBuffer`
7. *Separator*
8. Open Config… — no hint, `OpenConfig`
9. About vibeflow — no hint, `OpenRepoUrl`

Both builders are trivially unit-testable: assert ordering, separator placement, and enabled flags as a function of inputs.

### Layout & anchoring

`MenuLayout::compute(items, font_metrics, anchor, window_size) -> MenuLayout` is a pure function. It produces:
- `bbox: (x, y, w, h)` in physical pixels.
- `item_rects: Vec<(x, y, w, h)>` — one per `MenuItem`, in the same order.

Sizing rules:
- Item height = `font_cell_height + 4` px (top + bottom padding combined).
- Separator height = 1 px (rendered as a thin rect).
- Item width = `max(label_px + shortcut_hint_px + 32) over all items`, clamped to ≥ 220 px.
- Menu height = `8 + sum(item_heights) + 8` px (top + bottom padding).

Anchor flip rules (clamp to viewport):
- If `anchor.x + width > window_w`: place at `anchor.x - width` (right-align flip).
- If `anchor.y + height > window_h`: place at `anchor.y - height` (open upward).
- Both bounds clamped to ≥ 0.

If, after flips, the menu still wouldn't fit (window smaller than menu): clamp `(x, y) = (0, 0)` and accept clipping. Realistic only on tiny windows; degrades gracefully.

### Focus-index handling

`focus_next` / `focus_prev` advance/retreat past separators and disabled items, wrapping at the ends. Hover (CursorMoved over an enabled item) sets `focused = item_index` directly — keyboard and mouse stay unified.

### Hit testing

`MenuLayout::hit_test((x, y)) -> Option<HitRegion>` returns `Inside(item_idx)` when the cursor is over `item_rects[idx]`, else `Outside`. Used by both click handling (activate or dismiss) and CursorMoved (update focus).

### Dismissal

Menu closes on any of:
- `Esc` keypress.
- Left-click outside the menu's bbox. *Click is consumed; does NOT propagate to grid/tab.*
- Right-click anywhere — closes current menu before evaluating whether to open a new one.
- Item activation (Enter on focused, or click on enabled item).
- `WindowEvent::Focused(false)` — window loses focus.
- Window resize (anchor would be stale; recomputing position is fine but Stage 10 ships the simpler "close and let user re-open" behavior).
- Tab change (active session changes for any reason).
- Any non-menu keypress — closes, then the keypress is re-dispatched to grid input. Prevents an accidentally-open menu from swallowing typing.

### Rendering

In `render::context_menu::render(&self, …)`, after the bell-flash pass:

1. Background rect: solid fill of `[colors] menu_bg`, full menu bbox.
2. Border: four 1-px-thick rects forming an outline at the bbox edges, color `[colors] menu_border`.
3. For each item:
   - If `idx == focused`: focus highlight rect, color `[colors] menu_focus_bg`, item_rect bounds.
   - If `kind == Action`: label text quad (color `menu_text` or `menu_text_disabled` if `!enabled`), and if `shortcut_hint.is_some()`, hint text quad right-aligned (color `[colors] menu_shortcut`).
   - If `kind == Separator`: 1-px rect spanning width, color `menu_border`.

All shapes go through existing pipelines — no new shaders, no new bind groups.

### Blink-synced rename caret

The Stage 9 rename overlay's draw path (`render::tabs::draw_rename_overlay` or equivalent) gains a `cursor_blink: &CursorBlink` parameter. In the caret-rect emission step, gate on `cursor_blink.visible(now)`. The same `CursorBlink` instance is used for the active tab's terminal cursor, so the two render in identical on/off frames.

When the user has set `[cursor] blink_ms = 0`, `CursorBlink::visible` returns `true` unconditionally → the rename caret renders solid, matching the terminal cursor. No new config key needed.

### Config schema additions

`[colors]` section gains six keys, all optional, all default to dark-theme values:

```toml
[colors]
menu_bg            = "#1a1a22"
menu_border        = "#2a2a35"
menu_text          = "#e8e8ec"
menu_text_disabled = "#5a5a65"
menu_shortcut      = "#9999a5"
menu_focus_bg      = "#2a3550"
```

Hot-reload follows the Stage 9 pattern: notify watcher fires `ConfigReloaded`, `WindowApp::apply_config` updates the renderer's color cache, next redraw uses the new colors.

## Action specifics

Four menu actions need behavior pinned down:

**`Shortcut::SelectAll`** sets the selection to span the full grid buffer — including scrollback (negative line indices in alacritty_terminal). Even though Stage 10 does not render selection rectangles for scrollback rows (`render::mod.rs::build_selection_rects` already explicitly skips `p.line.0 < 0`), the selection is held end-to-end in the data model and copy reads through it. The user-visible win: a quick "select all → copy" gives the full session including invisible scrollback, which is the canonical terminal expectation.

Verified during the spec self-review: `SelectionTracker::cells()` and `SelectionTracker::text()` (in `crates/vibeflow/src/render/selection.rs`) already iterate the full selection range without filtering scrollback rows — `text()` reads through `term.grid()[p]` for any `Point`, and alacritty_terminal supports negative line indices for history. The only missing piece is a `select_all(&Term)` method that constructs the full-buffer Selection (start: `Point { line: -<history.len()>, column: 0 }`, end: `Point { line: <last visible row>, column: <last col> }`) plus a Stage 10 keymap entry mapping `Shortcut::SelectAll` to call it.

**`ClearBuffer`** writes a single byte `0x0c` (Ctrl+L) to the target PTY. The shell handles the rest — most shells redraw the prompt at the top with scrollback intact. Not `Term::clear_screen`, which would bypass the shell and look broken.

**`OpenConfig`** spawns `xdg-open ~/.config/vibeflow/config.toml` as a detached child via `std::process::Command::new("xdg-open").arg(path).spawn()`. The path is resolved from the existing config-load logic (which already knows it). GUI editor users get their preferred app; users without a `.toml` association can still open a new tab and edit there.

**`OpenRepoUrl`** spawns `xdg-open https://github.com/bjhengen/vibeflow`. Avoids needing a second overlay type for an info panel. The repo URL is hardcoded as a `const &str`.

For `OpenConfig` and `OpenRepoUrl`, errors (e.g., xdg-open not installed) are logged via `tracing::warn!` and the menu still closes. No user-visible error surface this stage; could be revisited in Stage 12 polish.

## Edge cases

- **Anchor near right/bottom edge:** flipped per anchor rules. Test points: `(0,0)`, `(window_w, window_h)`, `(window_w-10, window_h-10)`.
- **Disabled item activation:** Enter is impossible (focus skips disabled); click is a no-op (activation guard).
- **Separator activation:** click is a no-op (kind check); focus never lands on one.
- **Right-click during rename:** rename input commits its current text, closes, then the tab menu opens. Same rule Stage 9 uses for "any other interaction commits the rename."
- **Window resize while menu open:** menu closes. Recomputing on the fly is doable but adds state-management cost without proportional UX benefit.
- **Tab close while its tab menu is open:** if the action dispatches against a `target_session` that no longer exists, the action is dropped and the menu closes. No panic.
- **Modifier-only keypresses:** treated as "any other keypress" → menu closes, modifier event is re-dispatched to grid (consistent with Stage 8's modifier-press lesson). In practice, bare modifier presses produce no terminal bytes anyway, so the user-visible effect is just menu dismissal.
- **xdg-open absent / no association:** `tracing::warn!` logged; menu closes silently. User-visible degradation: nothing happens. Acceptable for v0.1.
- **`Clear Buffer` mid-command:** sends Ctrl+L while a TUI app (e.g., `vim`) is running — same effect as physically pressing Ctrl+L. User's responsibility.
- **Concurrent menu opens (right-click while menu already open):** sequential — existing menu closes, new one opens. Only ever one `ContextMenuState` at a time.

## Testing strategy

### Unit tests (in `render::context_menu`)

- `tab_menu(_, is_dead=true, _).len()` includes Restart Tab; `tab_menu(_, false, _)` excludes it.
- `tab_menu(_, _, tab_count=1)` renders Close Other Tabs as `enabled = false`.
- `grid_menu(has_selection=false)` renders Copy as `enabled = false`.
- `MenuLayout::compute`: anchor flip horizontal at right edge; flip vertical at bottom edge; both flips together at corner; degenerate clamp when window smaller than menu.
- `MenuLayout::compute`: item width clamped to ≥ 220 px when all labels are short.
- `focus_next` / `focus_prev`: wrap at boundaries; skip separators; skip disabled items.
- `MenuLayout::hit_test`: correct item index for known points; `Outside` for points beyond bbox.

### Unit tests (in `WindowApp`, with mocked PTY)

- Right-click on a tab → `context_menu.is_some()` with `target_session = Some(<that tab>)`.
- Right-click on grid → `context_menu.is_some()` with `target_session = None`.
- Right-click while menu open → previous menu closed, new one opened in the new location.
- Esc with menu open → `context_menu = None`.
- Left-click outside menu bbox → `context_menu = None`; no click forwarded to grid.
- `WindowEvent::Focused(false)` with menu open → `context_menu = None`.
- Tab close on the menu's `target_session` → subsequent activation is dropped.
- `MenuAction::Shortcut(Copy)` reaches the same handler as keyboard Ctrl+Shift+C (assert via spy).
- `MenuAction::ClearBuffer` writes `[0x0c]` to the target PTY (assert via mock writer).
- `MenuAction::CloseOtherTabs` reduces `sessions.len()` to 1 with the target intact.
- Rename + tab right-click sequence: rename input committed (text persisted on the tab), then tab menu opens.

### Integration tests (real PTY, no display required)

- Right-click → menu open → ArrowDown → ArrowDown → Enter on Paste → assert PTY received the clipboard bytes.
- Right-click on tab → Enter on Rename → keystroke 'x' → Enter → tab title contains 'x'.

### Manual smoke walk (host VNC)

Per the project's `feedback_vnc_display` lesson, GUI smoke runs are runnable on host.

1. Right-click in grid → menu appears at cursor; verify positioning, hover focus tracking, item enabled states.
2. Up/Down keyboard nav with no mouse motion; verify focus highlight moves and skips separators.
3. Click outside menu → dismisses without passing the click through to the grid.
4. Right-click near right edge → menu flips to open leftward.
5. Right-click near bottom edge → menu flips to open upward.
6. Right-click on a tab → tab menu opens with Rename first.
7. Activate Rename via menu → rename input opens with caret blinking in phase with the focused tab's terminal cursor (compare visually).
8. Set `[cursor] blink_ms = 0` in `~/.config/vibeflow/config.toml` → both terminal cursor and rename caret render solid.
9. Activate Open Config → user's GUI editor opens the config file (verify via process tree if needed).
10. Activate About vibeflow → browser opens the GitHub repo URL.
11. Activate Clear Buffer at a fresh shell prompt → prompt re-renders cleanly.
12. With 3 tabs open, activate Close Other Tabs → only the target tab remains.
13. Generate scrollback content (`seq 1 200`), then activate Select All → Copy → paste into another window → verify all 200 lines plus shell prompts are in the clipboard. Visual selection rectangles only show in the viewport; that's expected.
14. Type into the grid with no menu open → behavior unchanged from Stage 9 (regression check).

## Implementation sequencing (rough)

These are sketched here only to inform the writing-plans pass; the implementation plan will refine them.

1. `MenuItem`, `MenuAction`, `ItemKind` types + `tab_menu` / `grid_menu` builders + their unit tests.
2. `MenuLayout::compute` + anchor-flip math + width clamping + unit tests.
3. `MenuLayout::hit_test` + unit tests.
4. `ContextMenuState` + `focus_next` / `focus_prev` + unit tests.
5. `Shortcut::SelectAll` enum variant + default keybinding + Copy/Select-All consistency.
6. `[colors]` schema additions + defaults + hot-reload wiring.
7. `WindowApp::context_menu` field; right-click handler routes to grid- or tab-menu construction; tests via mocked PTY.
8. Input dispatch peeks at menu first; Up/Down/Enter/Esc/click handling; tests.
9. `MenuAction` dispatch: Shortcut variants reuse Stage 9 paths; `PastePrimary` / `ClearBuffer` / `CloseOtherTabs` / `OpenConfig` / `OpenRepoUrl` get small new handlers; tests.
10. `render::context_menu::render` — background, border, focus highlight, item text, separators; integrated into `Renderer::render`.
11. Rename overlay gains `&CursorBlink` parameter; caret rect gated on `cursor_blink.visible(now)`.
12. Integration tests (real PTY).
13. Senior pre-execution Sonnet review of the implementation plan against actual library/codebase source (see `feedback_senior_review_plans`).
14. Manual smoke walk on VNC; fix anything surfaced.
15. Senior holistic Sonnet review at end of stage.

## Risks & mitigations

- **Click-through bug on click-outside dismiss.** If left-click outside the menu accidentally dispatches to the grid, the user sees double action. Mitigation: explicit consume-the-click in the dismissal path; unit-tested.
- **Bare modifier presses dismissing menu.** Per Stage 8's lesson, bare modifier presses don't go through `key_to_bytes`. Treat them as "any other keypress" → close, but they emit no bytes so there's nothing to dispatch. Verify via test.
- **Blink phase drift if `&CursorBlink` is the wrong instance.** Test: open rename, observe both cursors blink on/off in lockstep at 1Hz; toggle `blink_ms` to 0 and 250 via config reload to confirm both follow.
- **xdg-open hangs on a misconfigured system.** Mitigated by `.spawn()` (detached, no `.wait()`), not `.status()`.
- **Menu render appearing under bell flash on collision.** Render order is explicit (bell flash before menu); won't happen unless the order is changed.

## Out-of-scope notes for future stages

- A generalized overlay subsystem (trait or enum) becomes worthwhile if a second overlay-style consumer arrives. Likely candidates: command palette (post-v0.1), in-buffer search (post-v0.1), per-tab notifications. At that point, refactor `ContextMenuState` + `RenameInputState` + `BellFlash` into the abstraction in a single dedicated stage.
- Tooltips and hover help are deferred. They would also be overlay-style and would benefit from the abstraction above.
- Sub-menus and cascading menus are out of scope; flat menus only.
- Drag-to-tear-off-tab and drag-to-reorder are unrelated to overlays and remain Stage 12 polish candidates.
