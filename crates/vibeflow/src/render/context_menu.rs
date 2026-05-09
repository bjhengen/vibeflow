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
