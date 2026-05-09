//! Stage 10: tactical context-menu overlay. State + layout + render lives in
//! this module; input wiring lives in `window.rs`. No generalized overlay
//! layer — see the Stage 10 design spec for the YAGNI rationale.

use alacritty_terminal::index::{Column, Line, Point};

use crate::keymap::Shortcut;

/// Resolved menu color palette. Written by `Renderer::set_menu_colors` from
/// the hot-reload path and read by `build_rects` (Task 13).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MenuColors {
    pub bg: [f32; 4],
    pub border: [f32; 4],
    pub text: [f32; 4],
    pub text_disabled: [f32; 4],
    pub shortcut: [f32; 4],
    pub focus_bg: [f32; 4],
}

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

    pub fn hit_test(&self, cursor: (f32, f32)) -> HitRegion {
        // Bbox check first (cheap rejection).
        let (bx, by, bw, bh) = self.bbox;
        if cursor.0 < bx || cursor.0 > bx + bw || cursor.1 < by || cursor.1 > by + bh {
            return HitRegion::Outside;
        }
        // Walk per-item rects in order.
        for (idx, &(rx, ry, rw, rh)) in self.item_rects.iter().enumerate() {
            if cursor.0 >= rx && cursor.0 <= rx + rw && cursor.1 >= ry && cursor.1 <= ry + rh {
                return HitRegion::Inside(idx);
            }
        }
        HitRegion::Outside
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
    /// separators and disabled items. No-op if no enabled action exists.
    pub fn focus_next(&mut self) {
        if let Some(idx) = self.find_enabled_action(self.focused, /* forward */ true) {
            self.focused = idx;
        }
    }

    /// Move focus to the previous enabled action item, wrapping at start.
    /// Skips separators and disabled items. No-op if no enabled action exists.
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

/// Build the rect instances for an open context menu — pushed onto the
/// caller's flat `all_rects` buffer in `Renderer::render`. Called LAST in
/// the rect-build sequence, after bell flash, so the menu sits above
/// every other layer.
///
/// Order within the returned Vec (the renderer must `draw_range` this
/// span as a single contiguous block):
///   1. background (full bbox)
///   2. four 1-px rects forming the border
///   3. per item: focus-highlight rect (only on focused index)
///   4. per separator: 1-px rect
pub fn build_rects(
    state: &ContextMenuState,
    colors: &MenuColors,
) -> Vec<crate::render::tabs::RectInstance> {
    let mut rects = Vec::with_capacity(8 + state.items.len());
    let (bx, by, bw, bh) = state.layout.bbox;
    // 1. Background.
    rects.push(crate::render::tabs::RectInstance::new(
        bx, by, bw, bh, colors.bg,
    ));
    // 2. Border (top, bottom, left, right — 1 px each).
    rects.push(crate::render::tabs::RectInstance::new(
        bx,
        by,
        bw,
        1.0,
        colors.border,
    ));
    rects.push(crate::render::tabs::RectInstance::new(
        bx,
        by + bh - 1.0,
        bw,
        1.0,
        colors.border,
    ));
    rects.push(crate::render::tabs::RectInstance::new(
        bx,
        by,
        1.0,
        bh,
        colors.border,
    ));
    rects.push(crate::render::tabs::RectInstance::new(
        bx + bw - 1.0,
        by,
        1.0,
        bh,
        colors.border,
    ));
    // 3. Focus highlight (only on focused index, only if Action and enabled).
    if let Some(item) = state.items.get(state.focused) {
        if matches!(item.kind, ItemKind::Action) && item.enabled {
            let (rx, ry, rw, rh) = state.layout.item_rects[state.focused];
            rects.push(crate::render::tabs::RectInstance::new(
                rx,
                ry,
                rw,
                rh,
                colors.focus_bg,
            ));
        }
    }
    // 4. Separators (1-px-tall rects at the item's y).
    for (idx, item) in state.items.iter().enumerate() {
        if matches!(item.kind, ItemKind::Separator) {
            let (rx, ry, rw, _rh) = state.layout.item_rects[idx];
            rects.push(crate::render::tabs::RectInstance::new(
                rx,
                ry,
                rw,
                1.0,
                colors.border,
            ));
        }
    }
    rects
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

    // ---- MenuLayout::compute ---------------------------------------------

    fn metrics() -> MenuFontMetrics {
        // 22 px tall items, 8 px wide chars (round numbers for assertions).
        MenuFontMetrics {
            item_height_px: 22.0,
            char_width_px: 8.0,
        }
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
        assert_eq!(
            s.focused, 3,
            "expected to skip separator (1) and disabled (2)"
        );
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
}
