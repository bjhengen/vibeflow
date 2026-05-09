# vibeflow Stage 10 — Overlay UX (right-click menus + blink-synced rename caret) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add right-click context menus on tab and grid areas with full keyboard navigation, plus blink-synced rename caret, by implementing a tactical `ContextMenuState` field on `WindowApp` (no generalized overlay layer).

**Architecture:** A new `render::context_menu` module owns `ContextMenuState`, `MenuItem`, `MenuLayout`, anchoring math, focus-index handling, and the menu render call. `WindowApp` gains an `Option<ContextMenuState>` field; right-click constructs it; input dispatch peeks at the menu before falling through to existing Stage 9 handling; activation produces a `MenuAction` that either reuses an existing `Shortcut` handler or dispatches a new menu-only handler. The Stage 9 rename overlay's caret rect is gated on the active tab's existing `CursorBlink` so both pulse in identical frames.

**Tech Stack:** Rust 1.x, winit 0.30, wgpu, alacritty_terminal 0.24, cosmic-text, existing `TabBarPipeline` (rects) + `QuadPipeline` + `TextEngine` (text quads). No new pipelines, no new external crates.

**Spec:** `docs/superpowers/specs/2026-05-09-vibeflow-stage10-overlay-design.md`

---

## Critical Stage 10 safety guards (re-state these in every implementer dispatch prompt)

Cheap implementers (Haiku) plow through these silently if they aren't pinned at the top of the dispatch prompt. Per the `feedback_implementer_safety` lesson, every dispatch must restate:

1. **DO NOT delete or weaken any existing test in any file you touch.** Adding tests is fine; modifying or removing existing tests is forbidden unless this plan's verbatim text authorizes it. Before reporting DONE, run:
   ```
   git show HEAD:<file> | grep -E '^\s*fn ' > /tmp/pre_fns.txt
   git show <your_sha>:<file> | grep -E '^\s*fn ' > /tmp/post_fns.txt
   diff /tmp/pre_fns.txt /tmp/post_fns.txt
   ```
   Any disappearing test names are a red flag — report BLOCKED.
2. **Report deviations honestly.** Even tiny ones — variable renames, removed `use` lines, swapped argv invocations, weakened assertions. The plan author wants visibility, not surprises.
3. **Cargo runs from the repo root.** Build/test/clippy commands target the workspace; do not `cd crates/vibeflow`.
4. **No `#[allow(dead_code)]` decay.** When this plan tells you to remove an `#[allow(dead_code)]` attribute as part of your task, you MUST do so. If clippy with `-D warnings` then fails because some other path still triggers dead_code, REPORT BLOCKED — do not silently re-add the allow.
5. **Quality gate per task:** `cargo fmt --all`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`. All four must pass before commit.

## Pre-execution senior review (workflow step, not a task)

Before dispatching the first implementer for Task 1, run a Sonnet-tier `general-purpose` review per the `feedback_senior_review_plans` lesson. Reviewer prompt sketch:

> Read `docs/superpowers/plans/2026-05-09-vibeflow-stage10-overlay.md`. Read the actual source it modifies — `crates/vibeflow/src/{render/{context_menu.rs (will not yet exist), mod.rs, tabs.rs, selection.rs, cursor.rs}, session/session.rs, app.rs, window.rs, keymap.rs, config/{schema.rs, mod.rs}}` — plus the alacritty_terminal 0.24 crate at `~/.cargo/registry/src/index.crates.io-*/alacritty_terminal-0.24.2/src/{term/mod.rs,index.rs,grid/mod.rs}` and winit 0.30 at the corresponding registry path. Verify every API claim, type signature, modifier name, struct field, and accessor existence in the plan. Categorize findings as Critical / Important / Minor / Verified-correct. Apply Critical fixes immediately; Important fixes unless the cost is high; Minor noted for the implementer.

Apply the review's fixes inline before T1 dispatch.

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `crates/vibeflow/src/render/context_menu.rs` | NEW | `ContextMenuState`, `MenuItem`, `MenuAction`, `ItemKind`, `MenuLayout`, `HitRegion`, `tab_menu`/`grid_menu` builders, focus + hit-test methods, render fn. All pure logic except the render fn. |
| `crates/vibeflow/src/render/mod.rs` | MODIFIED | `pub mod context_menu;` declaration; render-pass integration after bell flash. |
| `crates/vibeflow/src/render/tabs.rs` | MODIFIED | Rename overlay's draw path takes `&CursorBlink` and gates the caret rect on `cursor_blink.visible(now)`. |
| `crates/vibeflow/src/render/selection.rs` | MODIFIED | New `SelectionTracker::select_all(&Term)` method. |
| `crates/vibeflow/src/keymap.rs` | MODIFIED | New `Shortcut::SelectAll` enum variant; default keymap entry for Ctrl+Shift+A. |
| `crates/vibeflow/src/config/schema.rs` | MODIFIED | Six new optional menu color keys in `[colors]`; defaults; reload wiring. |
| `crates/vibeflow/src/window.rs` | MODIFIED | New `context_menu: Option<ContextMenuState>` field; right-click handler; menu input dispatch; `MenuAction` dispatch; `Shortcut::SelectAll` handler; rename overlay rendering threads `&CursorBlink`. |
| `crates/vibeflow/tests/context_menu.rs` | NEW | Integration tests against a real PTY. |

---

### Task 1: Scaffold `render::context_menu` module with types

**Files:**
- Create: `crates/vibeflow/src/render/context_menu.rs`
- Modify: `crates/vibeflow/src/render/mod.rs:1-15`

**Goal:** Land a compile-clean module with public types and stub builders. Subsequent tasks fill in logic and remove `#[allow(dead_code)]`.

- [ ] **Step 1: Create `crates/vibeflow/src/render/context_menu.rs` with the verbatim contents below.**

```rust
//! Stage 10: tactical context-menu overlay. State + layout + render lives in
//! this module; input wiring lives in `window.rs`. No generalized overlay
//! layer — see the Stage 10 design spec for the YAGNI rationale.

#![allow(dead_code)] // call sites land in Tasks 9–13; cleanup attribute removed in Task 9.

use alacritty_terminal::index::{Column, Line, Point};

use crate::keymap::Shortcut;

/// Tab index into `App.tabs()`. Defined locally as a type alias for clarity in
/// menu code without introducing a new newtype.
pub type SessionIdx = usize;

/// What the user can invoke from a menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Reuse Stage 9's existing per-shortcut handler. Tab-menu items pass
    /// the menu's `target_idx`; grid-menu items target the active tab.
    Shortcut(Shortcut),
    /// Read PRIMARY clipboard and write into the target PTY.
    PastePrimary,
    /// Write 0x0c (Ctrl+L) into the target PTY so the shell redraws.
    ClearBuffer,
    /// Close every tab except `target_idx` (or `App.active()` when None).
    CloseOtherTabs,
    /// Spawn `xdg-open <config_path>` detached.
    OpenConfig,
    /// Spawn `xdg-open <repo_url>` detached. URL is hardcoded.
    OpenRepoUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Action,
    Separator,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: &'static str,
    pub shortcut_hint: Option<&'static str>,
    pub action: MenuAction,
    pub enabled: bool,
    pub kind: ItemKind,
}

impl MenuItem {
    pub fn separator() -> Self {
        Self {
            label: "",
            shortcut_hint: None,
            action: MenuAction::Shortcut(Shortcut::NewTab), // unused for separators
            enabled: false,
            kind: ItemKind::Separator,
        }
    }
}

/// Pure-logic builder for the tab right-click menu.
pub fn tab_menu(_target_idx: SessionIdx, _is_dead: bool, _tab_count: usize) -> Vec<MenuItem> {
    Vec::new() // implemented in Task 2
}

/// Pure-logic builder for the grid right-click menu.
pub fn grid_menu(_has_selection: bool) -> Vec<MenuItem> {
    Vec::new() // implemented in Task 3
}

/// Pixel-space rectangle: (x, y, w, h).
pub type Rect = (f32, f32, f32, f32);

/// Computed layout for an open menu. Pure data; recomputed on open and on
/// window resize. `item_rects` is parallel to `MenuItem` order in the source
/// `Vec<MenuItem>`.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    pub bbox: Rect,
    pub item_rects: Vec<Rect>,
}

/// Where the cursor landed relative to a `MenuLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitRegion {
    Inside(usize),
    Outside,
}

/// Approximate font metrics needed to lay the menu out. Kept independent of
/// cosmic-text so tests don't need a `FontSystem`. The renderer fills these
/// from its actual font metrics at compute time.
#[derive(Debug, Clone, Copy)]
pub struct MenuFontMetrics {
    /// Vertical pitch of one item, in physical pixels (cell height + 4).
    pub item_height_px: f32,
    /// Approximate pixel width of one rendered character. Used for widths.
    /// Fine for fixed-width fonts; for proportional fonts it overshoots
    /// slightly, which is fine — the menu is wide enough either way.
    pub char_width_px: f32,
}

impl MenuLayout {
    /// Computed at open and on resize; reused for hit-testing each frame.
    pub fn compute(
        _items: &[MenuItem],
        _font: MenuFontMetrics,
        _anchor: (f32, f32),
        _window_size: (f32, f32),
    ) -> Self {
        Self {
            bbox: (0.0, 0.0, 0.0, 0.0),
            item_rects: Vec::new(),
        } // implemented in Task 4
    }

    pub fn hit_test(&self, _cursor: (f32, f32)) -> HitRegion {
        HitRegion::Outside // implemented in Task 5
    }
}

#[derive(Debug, Clone)]
pub struct ContextMenuState {
    /// Anchor in physical pixels (where the right-click happened). Kept so a
    /// resize can recompute the layout against the current window size.
    pub anchor: (f32, f32),
    pub items: Vec<MenuItem>,
    /// Index into `items` of the focused row. Always points at an enabled,
    /// non-separator item after open and after focus_next/focus_prev.
    pub focused: usize,
    /// Set when opened from a tab right-click; identifies which session this
    /// menu's actions target. None for grid menus → target = active tab.
    pub target_idx: Option<SessionIdx>,
    pub layout: MenuLayout,
}

impl ContextMenuState {
    /// Move focus to the next enabled action item, wrapping at end. Skips
    /// separators and disabled items. Implemented in Task 6.
    pub fn focus_next(&mut self) {
        let _ = self;
    }
    /// Move focus to the previous enabled action item, wrapping at start.
    /// Implemented in Task 6.
    pub fn focus_prev(&mut self) {
        let _ = self;
    }
}

/// Used by `SelectionTracker::select_all` (Task 7) to bound the selection's
/// upper end (`Point::new(Line(last_line), Column(last_col))`).
///
/// Re-exported only to keep the type bound visible in this module's docs. Not
/// constructed here.
#[allow(dead_code)]
pub(crate) type _MenuPoint = Point;
#[allow(dead_code)]
pub(crate) fn _menu_pt(line: i32, col: usize) -> Point {
    Point::new(Line(line), Column(col))
}
```

- [ ] **Step 2: Add the module declaration to `crates/vibeflow/src/render/mod.rs`.**

Find the existing module declarations near the top of `render/mod.rs` (mods like `pub mod tabs;`, `pub mod selection;`, `pub mod cursor;`, `pub mod text_engine;`, `pub mod quad;`, `pub mod bell;`, `pub mod mouse_encoder;`, `pub mod colors;`). Add this line in alphabetical position relative to the others:

```rust
pub mod context_menu;
```

- [ ] **Step 3: Verify the module compiles cleanly.**

Run from the repo root: `cargo build --workspace 2>&1 | tail -20`
Expected: build succeeds with zero warnings (the `#![allow(dead_code)]` covers everything in the new file).

- [ ] **Step 4: Run formatter + clippy.**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings 2>&1 | tail -20`
Expected: no errors, no warnings.

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs crates/vibeflow/src/render/mod.rs
git commit -m "feat(stage10): scaffold render::context_menu module with types"
```

---

### Task 2: `tab_menu` builder (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`

- [ ] **Step 1: Add a `#[cfg(test)] mod tests` block at the bottom of `crates/vibeflow/src/render/context_menu.rs` with these failing tests.**

Append (after the `_menu_pt` function):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Shortcut;

    fn assert_action(item: &MenuItem, label: &'static str, action: MenuAction) {
        assert_eq!(item.label, label, "label mismatch");
        assert_eq!(item.action, action, "action mismatch on {label}");
        assert_eq!(item.kind, ItemKind::Action, "expected Action kind on {label}");
    }

    fn assert_separator(item: &MenuItem) {
        assert_eq!(item.kind, ItemKind::Separator);
    }

    // ---- tab_menu ---------------------------------------------------------

    #[test]
    fn tab_menu_alive_excludes_restart_tab() {
        let items = tab_menu(0, /* is_dead */ false, /* tab_count */ 2);
        // Rename, ───, New Tab, ───, Close Tab, Close Other Tabs.
        assert_eq!(items.len(), 6);
        assert_action(&items[0], "Rename Tab", MenuAction::Shortcut(Shortcut::RenameTab));
        assert_separator(&items[1]);
        assert_action(&items[2], "New Tab", MenuAction::Shortcut(Shortcut::NewTab));
        assert_separator(&items[3]);
        assert_action(&items[4], "Close Tab", MenuAction::Shortcut(Shortcut::CloseTab));
        assert_action(&items[5], "Close Other Tabs", MenuAction::CloseOtherTabs);
    }

    #[test]
    fn tab_menu_dead_includes_restart_tab() {
        let items = tab_menu(0, /* is_dead */ true, /* tab_count */ 2);
        // Rename, Restart, ───, New Tab, ───, Close Tab, Close Other Tabs.
        assert_eq!(items.len(), 7);
        assert_action(&items[0], "Rename Tab", MenuAction::Shortcut(Shortcut::RenameTab));
        assert_action(&items[1], "Restart Tab", MenuAction::Shortcut(Shortcut::RestartTab));
        assert_separator(&items[2]);
        assert_action(&items[3], "New Tab", MenuAction::Shortcut(Shortcut::NewTab));
    }

    #[test]
    fn tab_menu_single_tab_disables_close_other_tabs() {
        let items = tab_menu(0, false, 1);
        let close_others = items.iter().find(|i| i.label == "Close Other Tabs").unwrap();
        assert!(!close_others.enabled, "Close Other Tabs must be disabled when only one tab");
    }

    #[test]
    fn tab_menu_multi_tab_enables_close_other_tabs() {
        let items = tab_menu(0, false, 3);
        let close_others = items.iter().find(|i| i.label == "Close Other Tabs").unwrap();
        assert!(close_others.enabled);
    }

    #[test]
    fn tab_menu_shortcut_hints() {
        let items = tab_menu(0, true, 2);
        let by_label = |l: &str| items.iter().find(|i| i.label == l).unwrap();
        assert_eq!(by_label("Rename Tab").shortcut_hint, Some("Ctrl+Shift+E"));
        assert_eq!(by_label("Restart Tab").shortcut_hint, Some("Ctrl+Shift+R"));
        assert_eq!(by_label("New Tab").shortcut_hint, Some("Ctrl+Shift+T"));
        assert_eq!(by_label("Close Tab").shortcut_hint, Some("Ctrl+Shift+W"));
        assert_eq!(by_label("Close Other Tabs").shortcut_hint, None);
    }
}
```

- [ ] **Step 2: Run the new tests; expect failures.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests 2>&1 | tail -25`
Expected: 5 failures, all because `tab_menu` returns an empty Vec.

- [ ] **Step 3: Replace the `tab_menu` stub in the same file with the real implementation.**

Replace the existing `tab_menu` function body. The new function:

```rust
pub fn tab_menu(target_idx: SessionIdx, is_dead: bool, tab_count: usize) -> Vec<MenuItem> {
    let _ = target_idx; // captured by ContextMenuState; not needed for item shape
    let mut items = Vec::with_capacity(7);
    items.push(MenuItem {
        label: "Rename Tab",
        shortcut_hint: Some("Ctrl+Shift+E"),
        action: MenuAction::Shortcut(Shortcut::RenameTab),
        enabled: true,
        kind: ItemKind::Action,
    });
    if is_dead {
        items.push(MenuItem {
            label: "Restart Tab",
            shortcut_hint: Some("Ctrl+Shift+R"),
            action: MenuAction::Shortcut(Shortcut::RestartTab),
            enabled: true,
            kind: ItemKind::Action,
        });
    }
    items.push(MenuItem::separator());
    items.push(MenuItem {
        label: "New Tab",
        shortcut_hint: Some("Ctrl+Shift+T"),
        action: MenuAction::Shortcut(Shortcut::NewTab),
        enabled: true,
        kind: ItemKind::Action,
    });
    items.push(MenuItem::separator());
    items.push(MenuItem {
        label: "Close Tab",
        shortcut_hint: Some("Ctrl+Shift+W"),
        action: MenuAction::Shortcut(Shortcut::CloseTab),
        enabled: true,
        kind: ItemKind::Action,
    });
    items.push(MenuItem {
        label: "Close Other Tabs",
        shortcut_hint: None,
        action: MenuAction::CloseOtherTabs,
        enabled: tab_count > 1,
        kind: ItemKind::Action,
    });
    items
}
```

- [ ] **Step 4: Run the tests; expect 5 passes.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::tab_menu 2>&1 | tail -10`
Expected: 5 passed.

- [ ] **Step 5: Run formatter + clippy + full test suite.**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
cargo test --workspace 2>&1 | tail -10
```
Expected: clippy clean; full suite all green.

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs
git commit -m "feat(stage10): tab_menu builder with Restart/Close-Other-Tabs gating"
```

---

### Task 3: `grid_menu` builder (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`

- [ ] **Step 1: Add the failing tests to the existing `mod tests` block (after the `tab_menu_*` tests).**

```rust
    // ---- grid_menu --------------------------------------------------------

    #[test]
    fn grid_menu_with_selection_enables_copy() {
        let items = grid_menu(/* has_selection */ true);
        // Copy, Paste, PastePrimary, ───, SelectAll, Clear, ───, OpenConfig, About.
        assert_eq!(items.len(), 9);
        let copy = items.iter().find(|i| i.label == "Copy").unwrap();
        assert!(copy.enabled);
    }

    #[test]
    fn grid_menu_without_selection_disables_copy() {
        let items = grid_menu(false);
        let copy = items.iter().find(|i| i.label == "Copy").unwrap();
        assert!(!copy.enabled);
    }

    #[test]
    fn grid_menu_item_order_and_actions() {
        let items = grid_menu(true);
        assert_action(&items[0], "Copy",            MenuAction::Shortcut(Shortcut::Copy));
        assert_action(&items[1], "Paste",           MenuAction::Shortcut(Shortcut::Paste));
        assert_action(&items[2], "Paste Selection", MenuAction::PastePrimary);
        assert_separator(&items[3]);
        assert_action(&items[4], "Select All",      MenuAction::Shortcut(Shortcut::SelectAll));
        assert_action(&items[5], "Clear Buffer",    MenuAction::ClearBuffer);
        assert_separator(&items[6]);
        assert_action(&items[7], "Open Config…",    MenuAction::OpenConfig);
        assert_action(&items[8], "About vibeflow",  MenuAction::OpenRepoUrl);
    }

    #[test]
    fn grid_menu_shortcut_hints() {
        let items = grid_menu(true);
        let by_label = |l: &str| items.iter().find(|i| i.label == l).unwrap();
        assert_eq!(by_label("Copy").shortcut_hint, Some("Ctrl+Shift+C"));
        assert_eq!(by_label("Paste").shortcut_hint, Some("Ctrl+Shift+V"));
        assert_eq!(by_label("Paste Selection").shortcut_hint, Some("Mid-click"));
        assert_eq!(by_label("Select All").shortcut_hint, Some("Ctrl+Shift+A"));
        assert_eq!(by_label("Clear Buffer").shortcut_hint, None);
        assert_eq!(by_label("Open Config…").shortcut_hint, None);
        assert_eq!(by_label("About vibeflow").shortcut_hint, None);
    }
```

These tests reference `Shortcut::SelectAll` which does not yet exist in the enum. Task 7 adds it. To keep this task self-contained and make the tests compile, do NOT pre-add `SelectAll` here — instead, in Step 2 you'll see compile errors, which is the expected red state.

- [ ] **Step 2: Run the tests; expect a *compile* failure.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::grid_menu 2>&1 | tail -15`
Expected: build error — `no variant or associated item named 'SelectAll' found for enum 'Shortcut'`.

- [ ] **Step 3: Add the `SelectAll` variant to `Shortcut` (forward-declared; full keymap binding lands in Task 7).**

Modify `crates/vibeflow/src/keymap.rs`. Find the `pub enum Shortcut { … }` definition. Append `SelectAll` at the end:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    RestartTab,
    Copy,
    Paste,
    /// Stage 9: open the inline rename input on the active tab.
    RenameTab,
    /// Stage 10: select the entire grid buffer (including scrollback). Default
    /// binding is Ctrl+Shift+A; wired to a real handler in Task 8.
    SelectAll,
}
```

This addition triggers a non-exhaustive `match` warning in `WindowApp::handle_shortcut` (window.rs ~line 292) and possibly in keymap.rs's own dispatch. Add a stub arm in **both** match sites so the project still compiles:

In `crates/vibeflow/src/window.rs`, find the `match shortcut { … }` inside `fn handle_shortcut`. Add a stub arm just before the closing brace:

```rust
            Shortcut::SelectAll => {
                // Wired in Task 8.
                tracing::debug!("SelectAll fired (handler lands in Task 8)");
            }
```

In `crates/vibeflow/src/keymap.rs`, search for any `match shortcut { … }` (look for `ShortcutTable::dispatch`-style code, or the `match` over `Shortcut::*` near the default keymap construction). If a match on `Shortcut` exists, add the corresponding `Shortcut::SelectAll => …` arm so the match stays exhaustive. If no match exists in keymap.rs (the file may only build a name → variant table), no change needed there.

- [ ] **Step 4: Run the grid_menu tests again; expect 4 failures.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::grid_menu 2>&1 | tail -10`
Expected: 4 failures, all because `grid_menu` returns an empty Vec.

- [ ] **Step 5: Replace the `grid_menu` stub in `context_menu.rs` with the real implementation.**

```rust
pub fn grid_menu(has_selection: bool) -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: "Copy",
            shortcut_hint: Some("Ctrl+Shift+C"),
            action: MenuAction::Shortcut(Shortcut::Copy),
            enabled: has_selection,
            kind: ItemKind::Action,
        },
        MenuItem {
            label: "Paste",
            shortcut_hint: Some("Ctrl+Shift+V"),
            action: MenuAction::Shortcut(Shortcut::Paste),
            enabled: true,
            kind: ItemKind::Action,
        },
        MenuItem {
            label: "Paste Selection",
            shortcut_hint: Some("Mid-click"),
            action: MenuAction::PastePrimary,
            enabled: true,
            kind: ItemKind::Action,
        },
        MenuItem::separator(),
        MenuItem {
            label: "Select All",
            shortcut_hint: Some("Ctrl+Shift+A"),
            action: MenuAction::Shortcut(Shortcut::SelectAll),
            enabled: true,
            kind: ItemKind::Action,
        },
        MenuItem {
            label: "Clear Buffer",
            shortcut_hint: None,
            action: MenuAction::ClearBuffer,
            enabled: true,
            kind: ItemKind::Action,
        },
        MenuItem::separator(),
        MenuItem {
            label: "Open Config…",
            shortcut_hint: None,
            action: MenuAction::OpenConfig,
            enabled: true,
            kind: ItemKind::Action,
        },
        MenuItem {
            label: "About vibeflow",
            shortcut_hint: None,
            action: MenuAction::OpenRepoUrl,
            enabled: true,
            kind: ItemKind::Action,
        },
    ]
}
```

- [ ] **Step 6: Run all context_menu tests + the full workspace suite.**

```bash
cargo test --package vibeflow --lib render::context_menu::tests 2>&1 | tail -10
cargo test --workspace 2>&1 | tail -15
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: 9 context_menu tests pass; full suite all green; clippy clean.

- [ ] **Step 7: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs crates/vibeflow/src/keymap.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage10): grid_menu builder + Shortcut::SelectAll variant (handler stubbed)"
```

---

### Task 4: `MenuLayout::compute` — sizing, anchor flips (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`

- [ ] **Step 1: Add failing tests to `mod tests`.**

Append:

```rust
    // ---- MenuLayout::compute ---------------------------------------------

    fn metrics() -> MenuFontMetrics {
        // 22 px tall items, 8 px wide chars (round numbers for assertions).
        MenuFontMetrics { item_height_px: 22.0, char_width_px: 8.0 }
    }

    fn long_items() -> Vec<MenuItem> {
        // Mix of action items and separators, with one wide label to drive
        // the width clamp away from the floor.
        vec![
            MenuItem {
                label: "A_very_wide_action_label_for_width",
                shortcut_hint: Some("Ctrl+Shift+E"),
                action: MenuAction::Shortcut(Shortcut::RenameTab),
                enabled: true,
                kind: ItemKind::Action,
            },
            MenuItem::separator(),
            MenuItem {
                label: "Short",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: true,
                kind: ItemKind::Action,
            },
        ]
    }

    fn short_items() -> Vec<MenuItem> {
        // All labels < 220 px to exercise the width floor clamp.
        vec![
            MenuItem {
                label: "Hi",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: true,
                kind: ItemKind::Action,
            },
            MenuItem {
                label: "Yo",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: true,
                kind: ItemKind::Action,
            },
        ]
    }

    #[test]
    fn layout_width_clamps_to_min_220() {
        let layout = MenuLayout::compute(&short_items(), metrics(), (10.0, 10.0), (1000.0, 1000.0));
        let (_, _, w, _) = layout.bbox;
        assert!(w >= 220.0, "width {w} < min 220");
    }

    #[test]
    fn layout_width_grows_for_long_items() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        let (_, _, w, _) = layout.bbox;
        // Wide label is 33 chars × 8 px = 264 px; shortcut hint is 12 chars × 8 = 96; gutter 32 → ~392 px total.
        assert!(w > 300.0, "expected wide menu, got {w}");
    }

    #[test]
    fn layout_height_sums_items_and_padding() {
        let items = long_items(); // 2 actions + 1 separator
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        let (_, _, _, h) = layout.bbox;
        // 2 * 22 (actions) + 1 * 1 (separator) + 8 + 8 (top/bottom pad) = 61.
        assert_eq!(h, 61.0);
    }

    #[test]
    fn layout_anchors_at_click_when_room_below_right() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (50.0, 60.0), (1000.0, 1000.0));
        let (x, y, _, _) = layout.bbox;
        assert_eq!((x, y), (50.0, 60.0));
    }

    #[test]
    fn layout_flips_horizontally_at_right_edge() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (980.0, 10.0), (1000.0, 1000.0));
        let (x, _, w, _) = layout.bbox;
        assert!(x < 980.0, "expected horizontal flip; bbox.x={x}");
        assert!(x + w <= 1000.0, "right edge {} > window 1000", x + w);
    }

    #[test]
    fn layout_flips_vertically_at_bottom_edge() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 990.0), (1000.0, 1000.0));
        let (_, y, _, h) = layout.bbox;
        assert!(y < 990.0, "expected vertical flip; bbox.y={y}");
        assert!(y + h <= 1000.0, "bottom {} > window 1000", y + h);
    }

    #[test]
    fn layout_clamps_to_zero_when_window_smaller_than_menu() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (5.0, 5.0), (50.0, 50.0));
        let (x, y, _, _) = layout.bbox;
        assert_eq!((x, y), (0.0, 0.0), "expected (0,0) clamp on tiny window");
    }

    #[test]
    fn layout_item_rects_align_with_bbox() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        assert_eq!(layout.item_rects.len(), items.len());
        let (bx, by, bw, _) = layout.bbox;
        // First item starts after the 8 px top padding.
        let (ix, iy, iw, ih) = layout.item_rects[0];
        assert_eq!(ix, bx);
        assert_eq!(iy, by + 8.0);
        assert_eq!(iw, bw);
        assert_eq!(ih, 22.0); // an action item
        // Separator's height is 1 px.
        let (_, _, _, sep_h) = layout.item_rects[1];
        assert_eq!(sep_h, 1.0);
    }
```

- [ ] **Step 2: Run the new tests; expect failures.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::layout 2>&1 | tail -25`
Expected: all 8 tests fail (the stub returns empty bbox + empty item_rects).

- [ ] **Step 3: Replace the `MenuLayout::compute` stub.**

```rust
impl MenuLayout {
    /// Pure-data layout computation. Padding constants, separator height, and
    /// gutter are stable across themes — colors are decoupled.
    pub fn compute(
        items: &[MenuItem],
        font: MenuFontMetrics,
        anchor: (f32, f32),
        window_size: (f32, f32),
    ) -> Self {
        const VPAD: f32 = 8.0;
        const GUTTER: f32 = 32.0;
        const MIN_WIDTH: f32 = 220.0;
        const SEPARATOR_H: f32 = 1.0;

        // Width: max(label + hint + gutter), clamped to floor.
        let mut max_text_w = 0.0_f32;
        for item in items {
            if matches!(item.kind, ItemKind::Separator) {
                continue;
            }
            let label_w = item.label.chars().count() as f32 * font.char_width_px;
            let hint_w = item
                .shortcut_hint
                .map(|h| h.chars().count() as f32 * font.char_width_px)
                .unwrap_or(0.0);
            max_text_w = max_text_w.max(label_w + hint_w);
        }
        let width = (max_text_w + GUTTER).max(MIN_WIDTH);

        // Item rects + total height accumulator.
        let mut item_rects = Vec::with_capacity(items.len());
        let mut y_cursor = anchor.1 + VPAD;
        let item_anchor_x = anchor.0;
        for item in items {
            let h = match item.kind {
                ItemKind::Action => font.item_height_px,
                ItemKind::Separator => SEPARATOR_H,
            };
            item_rects.push((item_anchor_x, y_cursor, width, h));
            y_cursor += h;
        }
        let height = (y_cursor - anchor.1) + VPAD;

        // Anchor flips. Compute desired (x, y), flip if overflow, then clamp.
        let mut x = anchor.0;
        let mut y = anchor.1;
        if x + width > window_size.0 {
            x = anchor.0 - width;
        }
        if y + height > window_size.1 {
            y = anchor.1 - height;
        }
        if x < 0.0 {
            x = 0.0;
        }
        if y < 0.0 {
            y = 0.0;
        }

        // If we shifted x or y, slide item_rects to match.
        let dx = x - anchor.0;
        let dy = (y + VPAD) - (anchor.1 + VPAD); // = y - anchor.1
        let item_rects: Vec<Rect> = item_rects
            .into_iter()
            .map(|(rx, ry, rw, rh)| (rx + dx, ry + dy, rw, rh))
            .collect();

        Self {
            bbox: (x, y, width, height),
            item_rects,
        }
    }

    pub fn hit_test(&self, _cursor: (f32, f32)) -> HitRegion {
        HitRegion::Outside // implemented in Task 5
    }
}
```

- [ ] **Step 4: Run the layout tests; expect 8 passes.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::layout 2>&1 | tail -10`
Expected: 8 passed.

- [ ] **Step 5: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: all green.

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs
git commit -m "feat(stage10): MenuLayout::compute with anchor flips + width clamp"
```

---

### Task 5: `MenuLayout::hit_test` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`

- [ ] **Step 1: Add failing tests.**

Append to `mod tests`:

```rust
    // ---- MenuLayout::hit_test --------------------------------------------

    #[test]
    fn hit_test_returns_inside_for_known_item() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        // Pick a point inside the first item's rect.
        let (rx, ry, rw, rh) = layout.item_rects[0];
        let cursor = (rx + rw / 2.0, ry + rh / 2.0);
        assert_eq!(layout.hit_test(cursor), HitRegion::Inside(0));
    }

    #[test]
    fn hit_test_returns_outside_above_bbox() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 100.0), (1000.0, 1000.0));
        assert_eq!(layout.hit_test((50.0, 50.0)), HitRegion::Outside);
    }

    #[test]
    fn hit_test_returns_outside_to_the_right_of_bbox() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        let (x, y, w, _) = layout.bbox;
        assert_eq!(layout.hit_test((x + w + 5.0, y + 5.0)), HitRegion::Outside);
    }

    #[test]
    fn hit_test_distinguishes_adjacent_items() {
        let items = long_items();
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        // Item 0 is an action (22 px), item 1 is a separator (1 px), item 2 is an action.
        let (rx, ry, _, _) = layout.item_rects[2];
        assert_eq!(layout.hit_test((rx + 5.0, ry + 5.0)), HitRegion::Inside(2));
    }
```

- [ ] **Step 2: Run; expect failures (current stub returns Outside always).**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::hit_test 2>&1 | tail -10`
Expected: 3 failures (the third one returns `Outside` instead of `Inside`; the `outside_above_bbox` test happens to pass because the stub always returns Outside).

- [ ] **Step 3: Replace the `hit_test` stub.**

```rust
    pub fn hit_test(&self, cursor: (f32, f32)) -> HitRegion {
        // Bbox check first (cheap rejection).
        let (bx, by, bw, bh) = self.bbox;
        if cursor.0 < bx || cursor.0 > bx + bw || cursor.1 < by || cursor.1 > by + bh {
            return HitRegion::Outside;
        }
        // Walk per-item rects in order.
        for (idx, &(rx, ry, rw, rh)) in self.item_rects.iter().enumerate() {
            if cursor.0 >= rx
                && cursor.0 <= rx + rw
                && cursor.1 >= ry
                && cursor.1 <= ry + rh
            {
                return HitRegion::Inside(idx);
            }
        }
        HitRegion::Outside
    }
```

- [ ] **Step 4: Run hit_test tests; expect all pass.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::hit_test 2>&1 | tail -10`
Expected: 4 passed.

- [ ] **Step 5: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs
git commit -m "feat(stage10): MenuLayout::hit_test with bbox + per-item rects"
```

---

### Task 6: `ContextMenuState::focus_next` / `focus_prev` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`

- [ ] **Step 1: Add failing tests.**

Append to `mod tests`:

```rust
    // ---- focus_next / focus_prev -----------------------------------------

    fn make_state_with_items(items: Vec<MenuItem>, initial_focus: usize) -> ContextMenuState {
        let layout = MenuLayout::compute(&items, metrics(), (10.0, 10.0), (1000.0, 1000.0));
        ContextMenuState {
            anchor: (10.0, 10.0),
            items,
            focused: initial_focus,
            target_idx: None,
            layout,
        }
    }

    fn nav_items() -> Vec<MenuItem> {
        // Action 0, Separator 1, Action 2 (disabled), Action 3, Action 4.
        vec![
            MenuItem {
                label: "A0",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: true,
                kind: ItemKind::Action,
            },
            MenuItem::separator(),
            MenuItem {
                label: "A2-disabled",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: false,
                kind: ItemKind::Action,
            },
            MenuItem {
                label: "A3",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: true,
                kind: ItemKind::Action,
            },
            MenuItem {
                label: "A4",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: true,
                kind: ItemKind::Action,
            },
        ]
    }

    #[test]
    fn focus_next_skips_separator_and_disabled() {
        let mut s = make_state_with_items(nav_items(), 0);
        s.focus_next();
        assert_eq!(s.focused, 3, "expected to skip separator (1) and disabled (2)");
    }

    #[test]
    fn focus_next_wraps_to_first_enabled() {
        let mut s = make_state_with_items(nav_items(), 4);
        s.focus_next();
        assert_eq!(s.focused, 0, "expected wrap to A0");
    }

    #[test]
    fn focus_prev_wraps_to_last_enabled() {
        let mut s = make_state_with_items(nav_items(), 0);
        s.focus_prev();
        assert_eq!(s.focused, 4, "expected wrap back to A4");
    }

    #[test]
    fn focus_prev_skips_separator_and_disabled() {
        let mut s = make_state_with_items(nav_items(), 3);
        s.focus_prev();
        assert_eq!(s.focused, 0, "expected skip A2-disabled and separator");
    }

    #[test]
    fn focus_next_on_single_enabled_item_is_idempotent() {
        let items = vec![MenuItem {
            label: "Only",
            shortcut_hint: None,
            action: MenuAction::ClearBuffer,
            enabled: true,
            kind: ItemKind::Action,
        }];
        let mut s = make_state_with_items(items, 0);
        s.focus_next();
        assert_eq!(s.focused, 0);
        s.focus_prev();
        assert_eq!(s.focused, 0);
    }

    #[test]
    fn focus_next_no_op_when_no_enabled_items() {
        // All disabled — defensive: don't loop forever.
        let items = vec![
            MenuItem {
                label: "Off1",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: false,
                kind: ItemKind::Action,
            },
            MenuItem::separator(),
            MenuItem {
                label: "Off2",
                shortcut_hint: None,
                action: MenuAction::ClearBuffer,
                enabled: false,
                kind: ItemKind::Action,
            },
        ];
        let mut s = make_state_with_items(items, 0);
        s.focus_next();
        // Should remain at 0 (no enabled item to move to).
        assert_eq!(s.focused, 0);
    }
```

- [ ] **Step 2: Run; expect failures.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::focus 2>&1 | tail -10`
Expected: 5 failures (the stub does nothing).

- [ ] **Step 3: Replace the focus stub methods.**

```rust
impl ContextMenuState {
    /// Move focus to the next enabled action item, wrapping at end. Skips
    /// separators and disabled items. No-op if no enabled action exists.
    pub fn focus_next(&mut self) {
        if let Some(idx) = self.find_enabled_action(self.focused, /* forward */ true) {
            self.focused = idx;
        }
    }

    /// Move focus to the previous enabled action item, wrapping at start.
    pub fn focus_prev(&mut self) {
        if let Some(idx) = self.find_enabled_action(self.focused, /* forward */ false) {
            self.focused = idx;
        }
    }

    fn find_enabled_action(&self, from: usize, forward: bool) -> Option<usize> {
        let n = self.items.len();
        if n == 0 {
            return None;
        }
        for step in 1..=n {
            let candidate = if forward {
                (from + step) % n
            } else {
                // Subtract `step` modulo `n` without underflow.
                (from + n - (step % n)) % n
            };
            let item = &self.items[candidate];
            if matches!(item.kind, ItemKind::Action) && item.enabled {
                return Some(candidate);
            }
        }
        None
    }
}
```

- [ ] **Step 4: Run; expect 6 passes.**

Run: `cargo test --package vibeflow --lib render::context_menu::tests::focus 2>&1 | tail -10`
Expected: 6 passed.

- [ ] **Step 5: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs
git commit -m "feat(stage10): ContextMenuState::focus_next/prev with sep + disabled skip"
```

---

### Task 7: `SelectionTracker::select_all` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/selection.rs`

- [ ] **Step 1: Add failing tests to the existing `mod tests` block in `selection.rs`.**

Append (after the last existing test):

```rust
    // ---- select_all (Stage 10) -------------------------------------------

    #[test]
    fn select_all_covers_visible_grid_when_no_history() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        t.select_all(&term);
        let s = t.current().expect("selection set after select_all");
        // start at (line: -history_size, col: 0); end at (line: 23, col: 79) for an 80x24 grid.
        assert_eq!(s.start.column.0, 0);
        assert_eq!(s.end.line.0, 23);
        assert_eq!(s.end.column.0, 79);
        // Default TermConfig has scrolling_history = 10000 → start.line = -10000.
        // Don't pin the exact value (config-dependent); assert it's <= 0.
        assert!(s.start.line.0 <= 0, "start.line should reach into history");
    }

    #[test]
    fn select_all_uses_cell_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        t.select_all(&term);
        let s = t.current().expect("selection set");
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    #[test]
    fn select_all_replaces_existing_selection() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        // Establish a small selection first.
        let now = Instant::now();
        t.mouse_down(pt(2, 3), false, &term, now);
        t.mouse_drag(pt(2, 8), &term);
        t.mouse_up();
        assert!(t.current().is_some());
        // Now select_all replaces it.
        t.select_all(&term);
        let s = t.current().expect("replaced");
        assert_eq!(s.end.line.0, 23);
        assert_eq!(s.end.column.0, 79);
    }
```

- [ ] **Step 2: Run; expect a compile error or failure.**

Run: `cargo test --package vibeflow --lib render::selection::tests::select_all 2>&1 | tail -15`
Expected: build error — `no method named 'select_all' found for struct 'SelectionTracker'`.

- [ ] **Step 3: Add the `select_all` method to `SelectionTracker`.**

In `crates/vibeflow/src/render/selection.rs`, find the `impl SelectionTracker { … }` block. Add this method (right after `pub fn clear` for proximity):

```rust
    /// Select the entire grid buffer including all available scrollback. The
    /// start line uses `-history_size as i32`; the end is the bottom-right
    /// cell of the viewport. `text()` and `cells()` already iterate the full
    /// range without filtering scrollback, so a subsequent copy retrieves the
    /// invisible history. Selection rectangles for scrollback rows are still
    /// filtered out of rendering by `build_selection_rects`.
    pub fn select_all(&mut self, term: &Term<VoidListener>) {
        let cols = term.columns();
        let lines = term.screen_lines();
        let history = term.history_size();
        let start = Point::new(Line(-(history as i32)), Column(0));
        let end = Point::new(Line(lines as i32 - 1), Column(cols.saturating_sub(1)));
        self.selection = Some(Selection {
            start,
            end,
            mode: SelectionMode::Cell,
        });
        self.drag_anchor = None;
    }
```

If the necessary imports aren't already in scope, ensure these are present at the top of `selection.rs` (most already are):

```rust
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::event::VoidListener;
```

`term.columns()` and `term.screen_lines()` are `Dimensions` trait methods — `Dimensions` is already imported in this file. `term.history_size()` is an inherent method on `Term`.

- [ ] **Step 4: Run select_all tests; expect 3 passes.**

Run: `cargo test --package vibeflow --lib render::selection::tests::select_all 2>&1 | tail -10`
Expected: 3 passed.

- [ ] **Step 5: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/render/selection.rs
git commit -m "feat(stage10): SelectionTracker::select_all covers full buffer + scrollback"
```

---

### Task 8: Wire `Shortcut::SelectAll` to a real handler + default keybinding

**Files:**
- Modify: `crates/vibeflow/src/window.rs`
- Modify: `crates/vibeflow/src/keymap.rs`

- [ ] **Step 1: Add a failing test asserting Ctrl+Shift+A invokes the handler.**

The right place depends on existing keymap test patterns. Check `keymap.rs` first: `grep -n '#\[test\]' crates/vibeflow/src/keymap.rs | head -10`. Add a test alongside the existing keymap tests:

```rust
    #[test]
    fn ctrl_shift_a_maps_to_select_all() {
        let table = ShortcutTable::default();
        // Match the helper or method existing tests use to look up a chord.
        // (Read existing tests in this file before adding — pattern may be
        // `table.lookup(ModifiersState::CONTROL | ModifiersState::SHIFT, "a")`
        // or `table.dispatch(...)`. Adapt the assertion below to the actual
        // helper's name and signature.)
        let mods = ModifiersState::CONTROL | ModifiersState::SHIFT;
        let action = table.lookup(mods, "a");
        assert_eq!(action, Some(Shortcut::SelectAll));
    }
```

If the existing test helper takes a `Key::Character` rather than a string, mirror that exact call shape — read the surrounding tests first and copy their pattern verbatim. Do NOT invent a new signature. If the keymap tests use `keymap::for_key(...)` or another function name, use that.

If you find that the closest existing test cannot be paralleled cleanly without knowledge of internal types, write the test against `WindowApp::handle_shortcut` directly instead. (Alternative test below if so.)

Alternative (in `window.rs` `mod tests`, if such a module exists; otherwise create `#[cfg(test)] mod tests` adjacent to `handle_shortcut`):

```rust
    #[test]
    fn handle_shortcut_select_all_sets_selection_on_active_tab() {
        // Construct a minimal WindowApp test fixture. Use whatever existing
        // window-tests pattern there is — if there is none, prefer extending
        // keymap.rs's test instead (since SelectAll wiring is testable
        // upstream of WindowApp's GUI dependencies).
    }
```

The keymap.rs test path is preferred — it avoids needing a winit/wgpu test fixture.

- [ ] **Step 2: Run the new test; expect failure.**

Run: `cargo test --package vibeflow --lib keymap 2>&1 | tail -15`
Expected: 1 failure on the new test (no binding for Ctrl+Shift+A yet).

- [ ] **Step 3: Add the default Ctrl+Shift+A binding.**

In `crates/vibeflow/src/keymap.rs`, find the place where existing default bindings are constructed (e.g., `impl Default for ShortcutTable` or a `default_bindings()` function). Add an entry mapping `Ctrl+Shift+A` to `Shortcut::SelectAll`. Match the exact construction style of existing entries (do not invent new field names or constructors).

- [ ] **Step 4: Replace the stub `Shortcut::SelectAll => { tracing::debug!(…) }` arm in `WindowApp::handle_shortcut` with a real handler.**

In `crates/vibeflow/src/window.rs`, replace the stub arm with:

```rust
            Shortcut::SelectAll => {
                let active = self.app.active();
                let Some(s) = self.app.tabs_mut().get_mut(active) else {
                    return;
                };
                s.selection.select_all(s.term());
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
```

- [ ] **Step 5: Run tests + workspace.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: all green; clippy clean.

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/keymap.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage10): Ctrl+Shift+A → Shortcut::SelectAll → SelectionTracker::select_all"
```

---

### Task 9: `[colors]` menu schema additions + defaults + hot-reload wiring

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs`
- Modify: `crates/vibeflow/src/render/mod.rs` or wherever indicator colors are stored on `Renderer` (look for `selection_color`, `indicator_colors`)
- Modify: `crates/vibeflow/src/render/context_menu.rs` (remove the file-level `#![allow(dead_code)]` — call sites land starting Task 10)
- Modify: `crates/vibeflow/src/window.rs` (extend `apply_config`)

- [ ] **Step 1: Find the existing color schema and existing renderer color setters as your model.**

Read these to learn the pattern: `grep -n 'selection\|indicator' crates/vibeflow/src/config/schema.rs | head -30` and `grep -n 'fn set_selection_color\|fn set_indicator_colors\|fn set_cursor_blink_ms' crates/vibeflow/src/render/mod.rs`.

The existing pattern: schema field `Option<[f32; 4]>` (or a typed wrapper); `Config::default_values` provides defaults; `Renderer` has a `set_…` method; `WindowApp::apply_config` calls each setter.

- [ ] **Step 2: Add menu color tests in `config/schema.rs`'s test block.**

```rust
    #[test]
    fn menu_colors_default_to_dark_theme_values() {
        let cf = Config::default();
        assert_eq!(cf.colors.menu_bg,            [0x1a as f32 / 255.0, 0x1a as f32 / 255.0, 0x22 as f32 / 255.0, 1.0]);
        assert_eq!(cf.colors.menu_border,        [0x2a as f32 / 255.0, 0x2a as f32 / 255.0, 0x35 as f32 / 255.0, 1.0]);
        assert_eq!(cf.colors.menu_text,          [0xe8 as f32 / 255.0, 0xe8 as f32 / 255.0, 0xec as f32 / 255.0, 1.0]);
        assert_eq!(cf.colors.menu_text_disabled, [0x5a as f32 / 255.0, 0x5a as f32 / 255.0, 0x65 as f32 / 255.0, 1.0]);
        assert_eq!(cf.colors.menu_shortcut,      [0x99 as f32 / 255.0, 0x99 as f32 / 255.0, 0xa5 as f32 / 255.0, 1.0]);
        assert_eq!(cf.colors.menu_focus_bg,      [0x2a as f32 / 255.0, 0x35 as f32 / 255.0, 0x50 as f32 / 255.0, 1.0]);
    }

    #[test]
    fn menu_colors_load_from_toml_overrides() {
        let toml = r#"
[colors]
menu_bg            = "#000000"
menu_border        = "#ffffff"
menu_text          = "#ffffff"
menu_text_disabled = "#888888"
menu_shortcut      = "#cccccc"
menu_focus_bg      = "#0000ff"
"#;
        let cf: Config = toml::from_str(toml).expect("parse");
        let cf = cf.with_defaults_filled();
        assert_eq!(cf.colors.menu_bg,            [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(cf.colors.menu_focus_bg,      [0.0, 0.0, 1.0, 1.0]);
    }
```

The exact API names (`with_defaults_filled`, `Config::default`) must match what schema.rs currently uses. Read existing tests; copy their exact accessor patterns. If `Config::default_values()` is the actual function name, use that. Reading the existing menu-color-test patterns is mandatory — do not invent.

- [ ] **Step 3: Run; expect 2 failures (fields don't exist).**

Run: `cargo test --package vibeflow --lib config::schema::tests::menu_colors 2>&1 | tail -15`
Expected: build error — fields missing.

- [ ] **Step 4: Add the six fields to the `[colors]` schema struct + defaults + parser plumbing.**

In `crates/vibeflow/src/config/schema.rs`, find the `Colors` struct (or whatever holds `selection`, `indicator_*`). Add:

```rust
    pub menu_bg: Option<[f32; 4]>,
    pub menu_border: Option<[f32; 4]>,
    pub menu_text: Option<[f32; 4]>,
    pub menu_text_disabled: Option<[f32; 4]>,
    pub menu_shortcut: Option<[f32; 4]>,
    pub menu_focus_bg: Option<[f32; 4]>,
```

(If the schema uses a non-Option type with serde defaults, mirror that pattern instead — read the file before editing.)

In the matching default-values function, add literals matching the test expectations above. The hex-to-`[f32;4]` conversion should use the same helper the existing color defaults use. If the existing pattern is `[0x0e as f32 / 255.0, …]`, use that.

- [ ] **Step 5: Add a renderer setter and wire it into `apply_config`.**

Pattern: existing `Renderer::set_selection_color` + `set_indicator_colors`. Add:

```rust
// in render/mod.rs (Renderer impl)
pub fn set_menu_colors(&mut self, colors: MenuColors) {
    self.menu_colors = colors;
}
```

Define a `pub struct MenuColors` (in `render::context_menu`, exported back through `render::mod.rs` if needed), holding the six `[f32; 4]` arrays. Add a `menu_colors: MenuColors` field to `Renderer` with defaults from `Config::default()`.

In `WindowApp::apply_config` (window.rs), add:

```rust
            r.set_menu_colors(crate::render::context_menu::MenuColors {
                bg:            config.colors.menu_bg,
                border:        config.colors.menu_border,
                text:          config.colors.menu_text,
                text_disabled: config.colors.menu_text_disabled,
                shortcut:      config.colors.menu_shortcut,
                focus_bg:      config.colors.menu_focus_bg,
            });
```

- [ ] **Step 6: Remove the file-level `#![allow(dead_code)]` from `render::context_menu.rs`.**

Delete the line `#![allow(dead_code)]` near the top. Build will likely surface a few unused-state warnings on `ContextMenuState` since no call site exists yet — keep `#[allow(dead_code)]` ONLY on the `ContextMenuState` struct itself and on `_MenuPoint`/`_menu_pt` (they're cleanup pieces). The remaining allows MUST be removed by Task 10 (when WindowApp acquires the field) and Task 13 (when the render fn lands). If you can't remove the allow without breaking the build, REPORT BLOCKED — do not silently re-add the file-level allow.

- [ ] **Step 7: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 8: Commit.**

```bash
git add crates/vibeflow/src/config/schema.rs crates/vibeflow/src/render/mod.rs crates/vibeflow/src/render/context_menu.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage10): [colors] menu_* keys + Renderer::set_menu_colors hot-reload"
```

---

### Task 10: `WindowApp.context_menu` field + right-click handler (open menu)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`
- Modify: `crates/vibeflow/src/render/context_menu.rs` (remove the struct-level `#[allow(dead_code)]`)

**Important:** Stage 9's tab-right-click → `start_rename` direct call (window.rs around line 781) is REPLACED in this task. The Stage 9 keyboard bindings (Ctrl+Shift+E / F2) for rename remain unchanged. Do not delete them.

- [ ] **Step 1: Skim the right-click site to understand the surrounding code.**

`sed -n '775,800p' crates/vibeflow/src/window.rs` — this is the existing `MouseInput Released, MouseButton::Right` arm.

- [ ] **Step 2: Add the field to `WindowApp`.**

In the `pub struct WindowApp { … }` definition, add:

```rust
    /// Stage 10: open right-click context menu, if any. At most one open.
    context_menu: Option<crate::render::context_menu::ContextMenuState>,
```

In `WindowApp::new` (or wherever the struct is constructed), initialize `context_menu: None` alongside the other fields.

- [ ] **Step 3: Add a small builder helper inside `WindowApp` impl.**

```rust
    /// Open a context menu anchored at (px_x, px_y). `target_idx` is `Some` for
    /// tab menus (set to the tab the user right-clicked) and `None` for grid
    /// menus (action targets the active tab).
    fn open_context_menu(&mut self, anchor: (f32, f32), target_idx: Option<usize>) {
        use crate::render::context_menu::{
            self, ContextMenuState, MenuFontMetrics, MenuLayout,
        };
        // Build items based on context.
        let items = match target_idx {
            Some(idx) => {
                let is_dead = self
                    .app
                    .tabs()
                    .get(idx)
                    .map(|s| s.is_dead())
                    .unwrap_or(true);
                let tab_count = self.app.tabs().len();
                context_menu::tab_menu(idx, is_dead, tab_count)
            }
            None => {
                let active = self.app.active();
                let has_selection = self
                    .app
                    .tabs()
                    .get(active)
                    .and_then(|s| s.selection.current())
                    .is_some();
                context_menu::grid_menu(has_selection)
            }
        };
        // Find the first enabled action for initial focus.
        let focused = items
            .iter()
            .position(|item| {
                matches!(item.kind, context_menu::ItemKind::Action) && item.enabled
            })
            .unwrap_or(0);
        // Compute layout. Font metrics come from the renderer (cell metrics).
        let (cell_w, cell_h) = self
            .renderer
            .as_ref()
            .map(|r| r.cell_size_px())
            .unwrap_or((8, 16));
        let font = MenuFontMetrics {
            item_height_px: cell_h as f32 + 4.0,
            char_width_px: cell_w as f32,
        };
        let window_size = self
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width as f32, s.height as f32)
            })
            .unwrap_or((1024.0, 768.0));
        let layout = MenuLayout::compute(&items, font, anchor, window_size);
        self.context_menu = Some(ContextMenuState {
            anchor,
            items,
            focused,
            target_idx,
            layout,
        });
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
```

`Renderer::cell_size_px()` and `PtySession::is_dead()` may already exist; if not, the senior pre-execution review should have caught it. If `is_dead()` doesn't exist, search for the field that tracks the death state of a `PtySession` (likely `s.dead` or `s.alive` or similar) and inline that boolean here. Same for `cell_size_px()`. Read the source — do not invent.

- [ ] **Step 4: Replace the existing right-click → rename block in the mouse handler.**

Before:

```rust
                    if state == ElementState::Released && button == MouseButton::Right {
                        // Stage 9: right-click on a tab body opens rename.
                        // …existing rename-open logic…
                    }
```

After:

```rust
                    if state == ElementState::Released && button == MouseButton::Right {
                        let Some((px, py)) = self.cursor_pos else { return; };
                        let anchor = (px as f32, py as f32);
                        // Hit-test against the tab strip.
                        let tab_idx = self
                            .renderer
                            .as_ref()
                            .and_then(|r| r.tab_bar_hit_test(px, py));
                        match tab_idx {
                            Some(idx) => self.open_context_menu(anchor, Some(idx)),
                            None => {
                                // If the click is in the grid (below the tab bar),
                                // open the grid menu. The hit-test for "below tab
                                // bar" is already implicit — `tab_bar_hit_test`
                                // returns None outside the bar.
                                self.open_context_menu(anchor, None);
                            }
                        }
                        return; // consumed
                    }
```

`Renderer::tab_bar_hit_test(px, py)` may need to be added (one-line wrapper around the existing tab-bar layout's hit-test). If the existing renderer already exposes a method that maps pixel coords to a tab index, use that exact name; otherwise add a thin wrapper. Read `render::tabs.rs` for the existing hit-test; many tab strips already expose this.

- [ ] **Step 5: Add unit tests for the open-menu state machine.**

If `WindowApp` already has a test module that constructs a fixture, add tests there. If not, prefer adding the tests in the form of pure-logic tests against `ContextMenuState` constructors that don't require WindowApp — but the open path goes through WindowApp, so a fixture is most accurate. If construction is too painful in a test, defer state-machine assertions to the integration tests in Task 14.

The minimum verifiable assertion in this task is: **the `cargo build`s clean and clippy is silent.** The state-machine tests for open/close land in Tasks 11–13 alongside the input handling.

- [ ] **Step 6: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 7: Commit.**

```bash
git add crates/vibeflow/src/window.rs crates/vibeflow/src/render/context_menu.rs crates/vibeflow/src/render/mod.rs crates/vibeflow/src/render/tabs.rs
git commit -m "feat(stage10): WindowApp.context_menu field + right-click opens tab/grid menu"
```

---

### Task 11: Menu input dispatch — keyboard navigation + dismissal

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Add the menu-first input branch at the top of `WindowApp`'s `KeyboardInput` handler.**

In the `WindowEvent::KeyboardInput { event, .. }` arm of `window_event`, BEFORE any existing keyboard logic that consumes the event, insert:

```rust
                if self.context_menu.is_some() {
                    use winit::keyboard::{Key, NamedKey};
                    if event.state == ElementState::Pressed {
                        match &event.logical_key {
                            Key::Named(NamedKey::ArrowDown) => {
                                if let Some(menu) = self.context_menu.as_mut() {
                                    menu.focus_next();
                                }
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                if let Some(menu) = self.context_menu.as_mut() {
                                    menu.focus_prev();
                                }
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::Enter) => {
                                self.activate_focused_menu_item();
                                return;
                            }
                            Key::Named(NamedKey::Escape) => {
                                self.context_menu = None;
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                            // Modifier-only presses keep the menu alive (per
                            // Stage 8 lesson: bare modifiers are key events
                            // too). Detect by checking that the key is one of
                            // the modifier NamedKeys.
                            Key::Named(
                                NamedKey::Control
                                | NamedKey::Shift
                                | NamedKey::Alt
                                | NamedKey::Super
                                | NamedKey::Meta,
                            ) => {
                                // Don't close on modifier-only press.
                            }
                            _ => {
                                // Any other typed key: close, then fall
                                // through to normal handling so the keystroke
                                // reaches the grid.
                                self.context_menu = None;
                            }
                        }
                    }
                }
```

`activate_focused_menu_item` is added in Task 12; for now, add a stub at the top of the `impl WindowApp` block:

```rust
    fn activate_focused_menu_item(&mut self) {
        // Implemented in Task 12.
        self.context_menu = None;
    }
```

- [ ] **Step 2: Add the menu-first branch for `CursorMoved` to track hover focus.**

In the `WindowEvent::CursorMoved { position, .. }` arm, after updating `cursor_pos`, add:

```rust
                if let Some(menu) = self.context_menu.as_mut() {
                    let cursor = (position.x as f32, position.y as f32);
                    if let crate::render::context_menu::HitRegion::Inside(idx) =
                        menu.layout.hit_test(cursor)
                    {
                        if matches!(menu.items[idx].kind, crate::render::context_menu::ItemKind::Action)
                            && menu.items[idx].enabled
                            && menu.focused != idx
                        {
                            menu.focused = idx;
                            if let Some(window) = self.window.as_ref() {
                                window.request_redraw();
                            }
                        }
                    }
                }
```

- [ ] **Step 3: Add `Focused(false)` and `Resized` dismissal hooks.**

In the relevant arms of `window_event`:

```rust
                WindowEvent::Focused(false) => {
                    if self.context_menu.is_some() {
                        self.context_menu = None;
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                    // … fall through to existing handling if any …
                }
                WindowEvent::Resized(_) => {
                    if self.context_menu.is_some() {
                        self.context_menu = None;
                    }
                    // … existing Resized handling …
                }
```

If a `Focused` arm doesn't exist yet, add one. Insert these BEFORE existing arms only if the existing arms are pattern-matching `_` catch-alls — otherwise the arms are mutually exclusive and order doesn't matter. Match the existing style.

- [ ] **Step 4: Run the workspace test suite to confirm nothing existing broke.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage10): menu input dispatch — kbd nav + focus loss + resize dismiss"
```

---

### Task 12: Menu input dispatch — mouse + `MenuAction` activation

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Replace the `activate_focused_menu_item` stub with a real implementation.**

```rust
    fn activate_focused_menu_item(&mut self) {
        let Some(menu) = self.context_menu.take() else { return; };
        let Some(item) = menu.items.get(menu.focused) else {
            return;
        };
        if !item.enabled || matches!(item.kind, crate::render::context_menu::ItemKind::Separator) {
            // Re-arm: defensive — should never happen if focus invariants hold.
            return;
        }
        let action = item.action;
        let target_idx = menu.target_idx;
        self.dispatch_menu_action(action, target_idx);
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
```

- [ ] **Step 2: Add the `dispatch_menu_action` method.**

```rust
    fn dispatch_menu_action(
        &mut self,
        action: crate::render::context_menu::MenuAction,
        target_idx: Option<usize>,
    ) {
        use crate::render::context_menu::MenuAction;
        match action {
            MenuAction::Shortcut(shortcut) => {
                // For tab-menu actions that target a specific tab, switch
                // active to it first so existing handlers (which key off
                // `App::active()`) operate against the right tab. The Stage 9
                // rename for tab N is the canonical case.
                if let Some(idx) = target_idx {
                    self.app.set_active(idx);
                }
                self.handle_shortcut(shortcut);
            }
            MenuAction::PastePrimary => {
                self.handle_paste_primary();
            }
            MenuAction::ClearBuffer => {
                let target = target_idx.unwrap_or_else(|| self.app.active());
                if let Some(s) = self.app.tabs_mut().get_mut(target) {
                    s.send_input(&[0x0c]);
                }
            }
            MenuAction::CloseOtherTabs => {
                let target = target_idx.unwrap_or_else(|| self.app.active());
                // Close from end to start so indices stay stable for `target`.
                let mut idx = self.app.tabs().len();
                while idx > 0 {
                    idx -= 1;
                    if idx != target {
                        self.app.close_tab(idx);
                    }
                }
                // After closing, the surviving tab is at index 0.
                if !self.app.tabs().is_empty() {
                    self.app.set_active(0);
                }
            }
            MenuAction::OpenConfig => {
                let path = self.config_path.clone();
                let _ = std::process::Command::new("xdg-open")
                    .arg(&path)
                    .spawn()
                    .map_err(|e| {
                        tracing::warn!("xdg-open {} failed: {e}", path.display());
                    });
            }
            MenuAction::OpenRepoUrl => {
                const REPO_URL: &str = "https://github.com/bjhengen/vibeflow";
                let _ = std::process::Command::new("xdg-open")
                    .arg(REPO_URL)
                    .spawn()
                    .map_err(|e| {
                        tracing::warn!("xdg-open {REPO_URL} failed: {e}");
                    });
            }
        }
    }
```

`PtySession::send_input(&[u8])` and `WindowApp::handle_paste_primary` may or may not already exist. Read window.rs and session.rs first. If `handle_paste_primary` doesn't exist as a separate method, factor it out from the existing `handle_paste` body — read existing paste plumbing, extract a `paste_bytes_to_active(&[u8])` helper, and have both handlers call it.

- [ ] **Step 3: Add the mouse-click handling in the existing `MouseInput` arm.**

Before any existing left-click logic, add:

```rust
                    if self.context_menu.is_some()
                        && state == ElementState::Released
                        && button == MouseButton::Left
                    {
                        let Some((px, py)) = self.cursor_pos else { return; };
                        let cursor = (px as f32, py as f32);
                        let menu = self.context_menu.as_ref().unwrap();
                        match menu.layout.hit_test(cursor) {
                            crate::render::context_menu::HitRegion::Inside(idx) => {
                                let item = &menu.items[idx];
                                if item.enabled && matches!(item.kind, crate::render::context_menu::ItemKind::Action) {
                                    // Reuse the keyboard activation path with focused = clicked.
                                    if let Some(menu) = self.context_menu.as_mut() {
                                        menu.focused = idx;
                                    }
                                    self.activate_focused_menu_item();
                                }
                                // Disabled or separator: no-op (menu stays open).
                                return;
                            }
                            crate::render::context_menu::HitRegion::Outside => {
                                // Dismiss; consume the click.
                                self.context_menu = None;
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                        }
                    }
```

For the right-click reopen behavior: in the existing right-click handler (Task 10), the menu is already replaced when a new right-click fires (`self.open_context_menu(...)` overwrites the old `self.context_menu`). No additional change here.

- [ ] **Step 4: Add a tab-close → dismiss hook.**

Find where `App::close_tab` is invoked from `handle_shortcut(Shortcut::CloseTab)`. After the close call, add:

```rust
                if self.context_menu.is_some() {
                    self.context_menu = None;
                }
```

(If close happens through other paths — child process exit etc — also dismiss the menu in those handlers. Search for all `close_tab` call sites.)

- [ ] **Step 5: Add a rename-commit-on-tab-right-click hook.**

In Task 10's `open_context_menu` for the tab-menu path, before constructing the menu state, add:

```rust
        // If a rename is in progress, commit it before opening the menu.
        if let Some(rename) = self.rename_state.take() {
            self.commit_rename(rename);
        }
```

`WindowApp::commit_rename(...)` may already exist as a Stage 9 helper. If not, factor it out from the existing rename-finalize path.

- [ ] **Step 6: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 7: Commit.**

```bash
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage10): MenuAction dispatch + click-to-activate + outside-click dismiss"
```

---

### Task 13: `render::context_menu::render` — wgpu integration

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`
- Modify: `crates/vibeflow/src/render/mod.rs` (call after bell-flash pass)

- [ ] **Step 1: Read existing rect and text-quad emit patterns.**

Bell flash and selection rects use `TabBarPipeline::queue_rect` (or similar) — read `render::bell.rs` and `render::tabs.rs` for the call shape. Tab labels use `text_engine` to push glyph quads into `quad_pipeline`. Match those exact patterns; do not invent.

`grep -n 'fn queue_rect\|fn push_glyph\|fn push_text_quads\|TextEngine::shape\|fn measure_str' crates/vibeflow/src/render/{bell.rs,tabs.rs,text_engine.rs,quad.rs} 2>&1 | head -30`

- [ ] **Step 2: Add the `render` function in `context_menu.rs`.**

Add at the end of `context_menu.rs`:

```rust
/// Render the open context menu. Called LAST in `Renderer::render`, after
/// the bell-flash overlay, so it sits above all other layers.
///
/// `tab_bar` and `text_engine` and `quad_pipeline` are mutable references to
/// the renderer's owned subsystems. The function pushes solid-color rects
/// (background, border, focus highlight, separators) into `tab_bar` and
/// glyph quads (item labels, shortcut hints) into `quad_pipeline`.
pub fn render(
    state: &ContextMenuState,
    colors: &MenuColors,
    tab_bar: &mut crate::render::tabs::TabBarPipeline,
    text_engine: &mut crate::render::text_engine::TextEngine,
    quad_pipeline: &mut crate::render::quad::QuadPipeline,
    cell_w: u32,
) {
    let _ = (state, colors, tab_bar, text_engine, quad_pipeline, cell_w);
    // Implementation matches the existing emit patterns in render::bell and
    // render::tabs. Read those files first; copy their exact call shapes for
    // pushing rects + text quads. Steps:
    //
    //   1. tab_bar.queue_rect(state.layout.bbox, colors.bg)
    //   2. four 1-px rects for the border (colors.border)
    //   3. for each item rect:
    //         - if focused: tab_bar.queue_rect(item_rect, colors.focus_bg)
    //         - if Action:
    //             - text quad for label at (item_rect.x + 14, item_rect.y + 4)
    //               using colors.text or colors.text_disabled
    //             - if shortcut_hint.is_some(): text quad right-aligned at
    //               (item_rect.x + item_rect.w - 14 - hint_w, ...)
    //         - if Separator: 1-px-tall rect at item_rect, color colors.border
    //
    // Concrete call shapes are codebase-specific and MUST be lifted verbatim
    // from `render::bell::draw` and `render::tabs::draw_tab_label`.
}

/// Color cache populated by `WindowApp::apply_config` from the `[colors]` schema keys.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MenuColors {
    pub bg: [f32; 4],
    pub border: [f32; 4],
    pub text: [f32; 4],
    pub text_disabled: [f32; 4],
    pub shortcut: [f32; 4],
    pub focus_bg: [f32; 4],
}
```

The body comment is intentionally directive: the actual rect/text emit calls have not been pinned down in the spec because the implementer must match the exact pipeline API in this codebase. The senior pre-execution review (workflow step before T1) MUST have flagged the exact method names; if not, this task's implementer should:

1. Read `render::bell.rs` and `render::tabs.rs` for the rect/text emit patterns.
2. Lift the *exact* method signatures and call shapes.
3. Implement the body without abstracting — straight-line code is easier to review.

If the implementer cannot find clear call shapes, REPORT BLOCKED with the call shapes encountered and the gaps observed.

- [ ] **Step 3: Wire the call into `Renderer::render`.**

Find the existing render pass in `render/mod.rs::Renderer::render`. After the bell-flash draw step, add:

```rust
        if let Some(menu) = context_menu {
            crate::render::context_menu::render(
                menu,
                &self.menu_colors,
                &mut self.tab_bar_pipeline,
                &mut self.text_engine,
                &mut self.quad_pipeline,
                cell_w,
            );
        }
```

`Renderer::render` will need to accept `context_menu: Option<&ContextMenuState>` as a parameter — add it to the signature alongside the existing parameters (`term`, `selection`, etc). Update the single call site in `WindowApp::request_redraw` (or wherever the render call originates) to pass `self.context_menu.as_ref()`.

- [ ] **Step 4: Smoke run on VNC to verify menu draws.**

```bash
# (on slmbeast VNC)
cargo build --release
RUST_LOG=vibeflow=info ./target/release/vibeflow &
# In a tab, right-click in the grid → menu should appear at the cursor.
# Verify: background, border, items, focus highlight, separator.
```

If the menu doesn't appear: re-check render-call ordering, `tab_bar.queue_rect` being flushed in this frame, and that `request_redraw()` is called when the menu opens.

- [ ] **Step 5: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 6: Commit.**

```bash
git add crates/vibeflow/src/render/context_menu.rs crates/vibeflow/src/render/mod.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage10): render context menu — bg + border + focus highlight + items"
```

---

### Task 14: Blink-synced rename caret

**Files:**
- Modify: `crates/vibeflow/src/render/tabs.rs`
- Modify: `crates/vibeflow/src/render/mod.rs` (caller)

- [ ] **Step 1: Read the existing rename-overlay draw path.**

`grep -n 'caret\|RenameInputState\|fn draw_rename\|pub fn render.*rename' crates/vibeflow/src/render/tabs.rs`

The rename overlay's caret is rendered as a 2-px-wide rect somewhere in the tab strip pass. Locate the function (likely `draw_rename_overlay` or inline within `draw_tab_strip`).

- [ ] **Step 2: Add a parameter to thread `&CursorBlink` through the draw call chain.**

Modify the draw function's signature to accept `cursor_blink: &crate::render::cursor::CursorBlink`. At the caret-emit step, gate:

```rust
        if cursor_blink.visible(now) {
            // existing 2-px rect emit for the caret
        }
```

`now: Instant` is already in scope at this site (the cursor blink uses it for the terminal cursor). Confirm and reuse it; do not introduce a second `now`.

Update the caller in `render::mod.rs::Renderer::render` to pass `&self.cursor`:

```rust
        // Existing rename-overlay draw call:
        crate::render::tabs::draw_rename_overlay(
            // … existing args …
            &self.cursor,
            now,
        );
```

If the existing draw call already has a `now` argument, reuse it as the same value passed to the terminal-cursor draw. Do NOT introduce a new `Instant::now()` here — Stage 9's existing pattern uses a single `now` per frame for both blink phases to be coherent.

- [ ] **Step 3: Smoke run on VNC.**

```bash
cargo build --release
RUST_LOG=vibeflow=info ./target/release/vibeflow &
```

In the running window:
1. Press Ctrl+Shift+E to open rename input on the active tab.
2. Visually compare: the rename caret and the active tab's terminal cursor should pulse in identical on/off frames at 1 Hz (default blink period).
3. Edit `~/.config/vibeflow/config.toml` to set `[cursor] blink_ms = 0`. Reload (just save). Both rename caret AND terminal cursor should now render solid.

- [ ] **Step 4: Quality gate.**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/render/tabs.rs crates/vibeflow/src/render/mod.rs
git commit -m "feat(stage10): rename caret blinks in phase with active tab's CursorBlink"
```

---

### Task 15: Integration tests (real PTY)

**Files:**
- Create: `crates/vibeflow/tests/context_menu.rs`

- [ ] **Step 1: Read existing integration tests for the test-fixture pattern.**

`ls crates/vibeflow/tests/ && head -60 crates/vibeflow/tests/$(ls crates/vibeflow/tests/ | head -1)`

Use the same `App` + spawn-shell + drive-events pattern.

- [ ] **Step 2: Create the integration test file with these tests.**

```rust
//! Stage 10 integration tests against a real PTY. Verify menu open/dispatch
//! flow end-to-end. No display required — these run headless.

// (Match the existing integration-tests preamble exactly. Read another file
// in this directory and copy the import block + any test-only helpers.)

use std::time::{Duration, Instant};
use vibeflow::app::App;
use vibeflow::render::context_menu::{
    self, ContextMenuState, MenuAction, MenuFontMetrics, MenuLayout,
};
use vibeflow::render::selection::SelectionTracker;

fn spawn_app_with_one_tab() -> App {
    let mut app = App::new();
    app.new_tab(&["bash"]).expect("spawn bash");
    app
}

fn drive_until(app: &mut App, deadline: Instant) {
    // Drive poll/tick like the main loop does. Match the existing
    // integration-test pattern from another file in this directory.
    while Instant::now() < deadline {
        let _ = app.poll(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn select_all_then_text_returns_full_buffer() {
    let mut app = spawn_app_with_one_tab();
    // Wait for the shell to print at least one prompt.
    drive_until(&mut app, Instant::now() + Duration::from_millis(300));
    let active = app.active();
    let s = &mut app.tabs_mut()[active];
    s.selection.select_all(s.term());
    let text = s.selection.text(s.term()).unwrap_or_default();
    assert!(!text.is_empty(), "select_all → text should produce shell output");
}

#[test]
fn grid_menu_disables_copy_when_no_selection() {
    let app = spawn_app_with_one_tab();
    let active = app.active();
    let has_sel = app.tabs()[active].selection.current().is_some();
    let items = context_menu::grid_menu(has_sel);
    let copy = items.iter().find(|i| i.label == "Copy").unwrap();
    assert!(!copy.enabled);
}

#[test]
fn tab_menu_for_dead_tab_includes_restart() {
    // Spawn a tab that exits immediately.
    let mut app = App::new();
    app.new_tab(&["true"]).expect("spawn true");
    // Drive until the child exits and the session marks dead.
    drive_until(&mut app, Instant::now() + Duration::from_millis(500));
    let is_dead = app.tabs()[0].is_dead();
    assert!(is_dead, "expected `true` child to have exited");
    let items = context_menu::tab_menu(0, is_dead, app.tabs().len());
    assert!(items.iter().any(|i| i.label == "Restart Tab"));
}
```

If the integration tests can't easily exercise the WindowApp path (winit + wgpu init), keep them at the App + selection level as above — that's the highest-value coverage that doesn't require a display.

- [ ] **Step 3: Run integration tests.**

```bash
cargo test --package vibeflow --tests 2>&1 | tail -15
```
Expected: all 3 (or however many) integration tests pass.

- [ ] **Step 4: Quality gate (full).**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
cargo build --release 2>&1 | tail -5
```

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/tests/context_menu.rs
git commit -m "test(stage10): integration tests for select_all + menu builders against real PTY"
```

---

## Manual smoke walk (after Task 15 passes)

Run on slmbeast VNC. Background-launch vibeflow so the loop returns:

```bash
cargo build --release
RUST_LOG=vibeflow=info ./target/release/vibeflow &
```

Walk through every item in the spec's "Manual smoke walk" section:
1. Right-click in grid → menu appears at cursor; verify positioning, hover focus tracking, item enabled states.
2. Up/Down keyboard nav with no mouse motion; verify focus highlight moves and skips separators.
3. Click outside menu → dismisses without passing the click through to the grid.
4. Right-click near right edge → menu flips to open leftward.
5. Right-click near bottom edge → menu flips to open upward.
6. Right-click on a tab → tab menu opens with Rename first.
7. Activate Rename via menu → rename input opens with caret blinking in phase with the focused tab's terminal cursor (compare visually).
8. Set `[cursor] blink_ms = 0` in `~/.config/vibeflow/config.toml` → both terminal cursor and rename caret render solid.
9. Activate Open Config → user's GUI editor opens the config file.
10. Activate About vibeflow → browser opens the GitHub repo URL.
11. Activate Clear Buffer at a fresh shell prompt → prompt re-renders cleanly.
12. With 3 tabs open, activate Close Other Tabs → only the target tab remains.
13. Generate scrollback content (`seq 1 200`), then activate Select All → Copy → paste into another window → verify all 200 lines plus shell prompts are in the clipboard.
14. Type into the grid with no menu open → behavior unchanged from Stage 9 (regression check).

Fix any issues found. Commit fixes individually. Each fix gets its own conventional-commit message.

## Senior holistic review (after smoke walk)

Per the `lesson_subagent_workflow_at_scale` Stage 9 lesson, dispatch a final Sonnet-tier holistic review at end of stage. Reviewer prompt:

> Read the Stage 10 plan, spec, and every commit on this branch. Identify two classes of issue: (a) design-level mistakes that span files (the kind a per-task reviewer can't see) and (b) cross-task consistency drift (renamed types, mismatched method signatures, divergent state-machine assumptions). Report Critical / Important / Minor.

Apply Critical fixes; apply Important unless cost is high; note Minor.

## Plan self-review checklist

Spec coverage:
- [x] Right-click on tab → tab menu (T10)
- [x] Right-click on grid → grid menu (T10)
- [x] Tab menu items + Restart-only-when-dead + Close-Other-Tabs disabled when count=1 (T2)
- [x] Grid menu items + Copy disabled when no selection (T3)
- [x] Full keyboard nav: Up/Down/Enter/Esc (T11)
- [x] Click outside dismisses without passing through (T12)
- [x] Right-click reopens (T10 + T12 — open_context_menu always replaces)
- [x] CursorMoved updates focus (T11)
- [x] Window focus loss / Resize / tab close → dismiss (T11 + T12)
- [x] Modifier-only keypress doesn't dismiss (T11 — special-cased)
- [x] Phase-locked rename caret via shared CursorBlink (T14)
- [x] Shortcut::SelectAll variant + binding (T3 + T7 + T8)
- [x] SelectionTracker::select_all (T7)
- [x] Six [colors] menu_* keys + defaults + hot-reload (T9)
- [x] MenuAction dispatch for Shortcut + PastePrimary + ClearBuffer + CloseOtherTabs + OpenConfig + OpenRepoUrl (T12)
- [x] Layout: anchor flips at right/bottom, width clamp ≥ 220 px, height = sum + 2×padding (T4)
- [x] Hit testing for click + hover (T5)
- [x] focus_next/prev skips separators + disabled (T6)
- [x] Render order: bell flash → context menu (T13)
- [x] Integration tests (T15)
- [x] Manual smoke walk (post-T15)

Forward-declared item lifecycle:
- T1: file-level `#![allow(dead_code)]` — removed in T9.
- T1: per-struct `#[allow(dead_code)]` on `ContextMenuState` — removed in T10 when WindowApp gains the field.
- T1: `_MenuPoint` / `_menu_pt` cleanup helpers with allows — kept; can be deleted by an end-of-stage commit if unused.

Cross-task type consistency:
- `MenuAction`, `MenuItem`, `ItemKind`, `MenuLayout`, `HitRegion`, `ContextMenuState`, `MenuColors`, `MenuFontMetrics` — all defined in T1 with the exact fields used in T2–T15.
- `Shortcut::SelectAll` — added in T3 (with stub arm), wired in T8.
- `MenuColors` — defined in T13 with the field names referenced in T9's `apply_config` call (bg/border/text/text_disabled/shortcut/focus_bg).

No placeholders found except for genuinely codebase-specific call shapes (T9 `Config::default_values` API, T13 wgpu emit patterns, T14 existing rename draw fn name) — these are flagged in-place with "read existing source" instructions and a "REPORT BLOCKED if you can't find them" escape hatch. The senior pre-execution review (workflow step) is the catch-all for verifying these.
