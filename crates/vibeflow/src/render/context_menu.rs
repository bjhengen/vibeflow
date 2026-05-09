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
pub fn tab_menu(_target_idx: SessionIdx, is_dead: bool, tab_count: usize) -> Vec<MenuItem> {
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

/// Pure-logic builder for the grid right-click menu.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Shortcut;

    fn assert_action(item: &MenuItem, label: &'static str, action: MenuAction) {
        assert_eq!(item.label, label, "label mismatch");
        assert_eq!(item.action, action, "action mismatch on {label}");
        assert_eq!(
            item.kind,
            ItemKind::Action,
            "expected Action kind on {label}"
        );
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
        assert_action(
            &items[0],
            "Rename Tab",
            MenuAction::Shortcut(Shortcut::RenameTab),
        );
        assert_separator(&items[1]);
        assert_action(&items[2], "New Tab", MenuAction::Shortcut(Shortcut::NewTab));
        assert_separator(&items[3]);
        assert_action(
            &items[4],
            "Close Tab",
            MenuAction::Shortcut(Shortcut::CloseTab),
        );
        assert_action(&items[5], "Close Other Tabs", MenuAction::CloseOtherTabs);
    }

    #[test]
    fn tab_menu_dead_includes_restart_tab() {
        let items = tab_menu(0, /* is_dead */ true, /* tab_count */ 2);
        // Rename, Restart, ───, New Tab, ───, Close Tab, Close Other Tabs.
        assert_eq!(items.len(), 7);
        assert_action(
            &items[0],
            "Rename Tab",
            MenuAction::Shortcut(Shortcut::RenameTab),
        );
        assert_action(
            &items[1],
            "Restart Tab",
            MenuAction::Shortcut(Shortcut::RestartTab),
        );
        assert_separator(&items[2]);
        assert_action(&items[3], "New Tab", MenuAction::Shortcut(Shortcut::NewTab));
    }

    #[test]
    fn tab_menu_single_tab_disables_close_other_tabs() {
        let items = tab_menu(0, false, 1);
        let close_others = items
            .iter()
            .find(|i| i.label == "Close Other Tabs")
            .unwrap();
        assert!(
            !close_others.enabled,
            "Close Other Tabs must be disabled when only one tab"
        );
    }

    #[test]
    fn tab_menu_multi_tab_enables_close_other_tabs() {
        let items = tab_menu(0, false, 3);
        let close_others = items
            .iter()
            .find(|i| i.label == "Close Other Tabs")
            .unwrap();
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
        assert_action(&items[0], "Copy", MenuAction::Shortcut(Shortcut::Copy));
        assert_action(&items[1], "Paste", MenuAction::Shortcut(Shortcut::Paste));
        assert_action(&items[2], "Paste Selection", MenuAction::PastePrimary);
        assert_separator(&items[3]);
        assert_action(
            &items[4],
            "Select All",
            MenuAction::Shortcut(Shortcut::SelectAll),
        );
        assert_action(&items[5], "Clear Buffer", MenuAction::ClearBuffer);
        assert_separator(&items[6]);
        assert_action(&items[7], "Open Config…", MenuAction::OpenConfig);
        assert_action(&items[8], "About vibeflow", MenuAction::OpenRepoUrl);
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
}
