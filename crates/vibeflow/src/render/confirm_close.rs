//! v0.1.3: Confirm-on-close modal overlay — layout + rect/glyph builders.
//!
//! Mirrors `render/about.rs`: pure layout + render builders live here; input
//! wiring and state machine live in `window.rs`. No new render pass — rects
//! flow through `TabBarPipeline`; glyphs through `QuadPipeline`.

use crate::app::BusyTabInfo;

/// What's currently focused for keyboard activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedButton {
    Cancel,
    CloseAnyway,
}

/// v0.1.3 per-tab close amendment: which close action the dialog is gating.
/// Drives title text, confirm-button label, and (in `window.rs`) which close
/// operation to perform when the user confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmCloseScope {
    /// Whole window: equivalent to `WindowEvent::CloseRequested`.
    Window,
    /// Closing one tab (Ctrl+W or X-button click). 0-based App index.
    SingleTab { tab_index: usize },
    /// Closing every tab except `keep_tab_index` (context-menu action).
    OtherTabs { keep_tab_index: usize },
}

/// State carried while the dialog is open. Captured at open time and
/// immutable thereafter (snapshot of busy tabs at that moment).
#[derive(Debug, Clone)]
pub struct ConfirmCloseState {
    pub busy_tabs: Vec<BusyTabInfo>,
    /// Number of tabs the dialog action would close if Confirm were clicked.
    /// - `Window`: total tabs.
    /// - `SingleTab`: 1.
    /// - `OtherTabs(keep)`: total − 1.
    pub tab_count: usize,
    pub focus: FocusedButton,
    pub scope: ConfirmCloseScope,
}

impl ConfirmCloseState {
    /// Capture state at dialog-open time for the window-close path. Cancel
    /// is the default focus — muscle-memory Enter spam shouldn't kill
    /// in-flight work. Backwards-compatible with the v0.1.3 §3 spec; defaults
    /// `scope = Window`.
    pub fn new(busy_tabs: Vec<BusyTabInfo>, tab_count: usize) -> Self {
        Self::with_scope(busy_tabs, tab_count, ConfirmCloseScope::Window)
    }

    /// v0.1.3 per-tab close amendment: same as `new` but caller selects the
    /// dialog scope (single-tab close, "close other tabs", or whole window).
    pub fn with_scope(
        busy_tabs: Vec<BusyTabInfo>,
        tab_count: usize,
        scope: ConfirmCloseScope,
    ) -> Self {
        Self {
            busy_tabs,
            tab_count,
            focus: FocusedButton::Cancel,
            scope,
        }
    }

    /// Cycle focus: Cancel ↔ CloseAnyway. Used by Tab / Shift+Tab / arrows.
    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            FocusedButton::Cancel => FocusedButton::CloseAnyway,
            FocusedButton::CloseAnyway => FocusedButton::Cancel,
        };
    }

    /// True when the dialog is in "busy mode" (≥1 busy tab — show list).
    /// False when in "multi-tab all idle mode" (just a count + Close all?).
    pub fn is_busy_mode(&self) -> bool {
        !self.busy_tabs.is_empty()
    }
}

/// v0.1.3 per-tab amendment: title text shown at the top of the dialog.
pub fn title_text(state: &ConfirmCloseState) -> &'static str {
    match state.scope {
        ConfirmCloseScope::Window => "Close vibeflow?",
        ConfirmCloseScope::SingleTab { .. } => "Close this tab?",
        ConfirmCloseScope::OtherTabs { .. } => "Close other tabs?",
    }
}

/// v0.1.3 per-tab amendment: label for the destructive (right) button.
pub fn confirm_button_label(state: &ConfirmCloseState) -> &'static str {
    match state.scope {
        ConfirmCloseScope::Window => "Close anyway",
        ConfirmCloseScope::SingleTab { .. } => "Close tab",
        ConfirmCloseScope::OtherTabs { .. } => "Close other tabs",
    }
}

/// Resolved palette for the confirm-close overlay. Same shape as `AboutColors`
/// plus button-fill variants for the focused / unfocused button states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfirmCloseColors {
    pub backdrop: [f32; 4],
    pub panel_bg: [f32; 4],
    pub border_fg: [f32; 4],
    pub text_fg: [f32; 4],
    /// Idle button background. Borrow `menu_bg` from the theme.
    pub button_idle_bg: [f32; 4],
    /// Focused button background (Cancel by default; switches on Tab).
    pub button_focus_bg: [f32; 4],
    /// Button label text colour. Same as `text_fg` in practice; named
    /// separately so a future theme can darken text on focus if desired.
    pub button_text_fg: [f32; 4],
}

/// `(x, y, w, h)` of the centred dialog panel in logical pixels.
///
/// Sizing rules (mirror About):
/// - Busy mode default: 640×320 px (room for ~8 list rows + buttons).
/// - Idle-multi-tab mode default: 480×200 px (no list region).
/// - Clamp to `window - 40` on each axis when smaller than (default + 80).
/// - 8 px margin floor at very tiny windows; zero-sized window → zero panel.
pub fn panel_rect(window_size: (u32, u32), state: &ConfirmCloseState) -> (f32, f32, f32, f32) {
    let (default_w, default_h) = if state.is_busy_mode() {
        (640.0_f32, 320.0_f32)
    } else {
        (480.0_f32, 200.0_f32)
    };
    let clamp_threshold_w = default_w + 80.0;
    let clamp_threshold_h = default_h + 60.0;
    const TINY_THRESHOLD_W: f32 = 200.0;
    const TINY_THRESHOLD_H: f32 = 120.0;
    const STANDARD_MARGIN: f32 = 20.0;
    const TINY_MARGIN: f32 = 8.0;

    let window_w = window_size.0 as f32;
    let window_h = window_size.1 as f32;

    let margin = if window_w < TINY_THRESHOLD_W || window_h < TINY_THRESHOLD_H {
        TINY_MARGIN
    } else {
        STANDARD_MARGIN
    };

    let w = if window_w < clamp_threshold_w {
        (window_w - 2.0 * margin).max(0.0)
    } else {
        default_w
    };
    let h = if window_h < clamp_threshold_h {
        (window_h - 2.0 * margin).max(0.0)
    } else {
        default_h
    };

    let x = ((window_w - w) / 2.0).max(0.0);
    let y = ((window_h - h) / 2.0).max(0.0);
    (x, y, w, h)
}

/// Button geometry. Returned by `button_rects` and consumed by both the
/// rect-builder (for drawing) and `hit_test_buttons` (for click routing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonGeom {
    pub cancel: (f32, f32, f32, f32), // x, y, w, h
    pub close_anyway: (f32, f32, f32, f32),
}

/// Compute button rectangles relative to the panel. Cancel on left, Close
/// anyway on right; both anchored to the panel's bottom edge with 16 px gap
/// between them and 16 px from the panel borders.
pub fn button_rects(panel: (f32, f32, f32, f32)) -> ButtonGeom {
    const BUTTON_W: f32 = 140.0;
    const BUTTON_H: f32 = 36.0;
    const BOTTOM_GAP: f32 = 16.0;
    const BETWEEN: f32 = 16.0;
    const SIDE_PAD: f32 = 16.0;
    let (px, py, pw, ph) = panel;
    let by = py + ph - BUTTON_H - BOTTOM_GAP;
    // Right-anchor the pair so they sit at the bottom-right of the panel.
    let cax = px + pw - SIDE_PAD - BUTTON_W;
    let cx = cax - BETWEEN - BUTTON_W;
    ButtonGeom {
        cancel: (cx, by, BUTTON_W, BUTTON_H),
        close_anyway: (cax, by, BUTTON_W, BUTTON_H),
    }
}

#[cfg(test)]
mod tests_layout {
    use super::*;
    use crate::app::BusyTabInfo;

    fn busy(n: usize) -> Vec<BusyTabInfo> {
        (0..n)
            .map(|i| BusyTabInfo {
                tab_index: i + 1,
                display_label: format!("proc{i}"),
                state_label: "running".to_string(),
            })
            .collect()
    }

    fn busy_state(n: usize, total: usize) -> ConfirmCloseState {
        ConfirmCloseState::new(busy(n), total)
    }

    #[test]
    fn panel_rect_busy_mode_default_is_640_320() {
        let s = busy_state(2, 3);
        let (_, _, w, h) = panel_rect((1920, 1080), &s);
        assert_eq!((w, h), (640.0, 320.0));
    }

    #[test]
    fn panel_rect_idle_multi_tab_mode_default_is_480_200() {
        let s = busy_state(0, 3);
        let (_, _, w, h) = panel_rect((1920, 1080), &s);
        assert_eq!((w, h), (480.0, 200.0));
    }

    #[test]
    fn panel_rect_centres_in_window() {
        let s = busy_state(2, 3);
        let (x, y, w, h) = panel_rect((1920, 1080), &s);
        assert_eq!(x + w / 2.0, 960.0);
        assert_eq!(y + h / 2.0, 540.0);
    }

    #[test]
    fn panel_rect_clamps_in_small_window() {
        let s = busy_state(2, 3);
        let (_, _, w, h) = panel_rect((500, 300), &s);
        assert_eq!(w, 460.0);
        assert_eq!(h, 260.0);
    }

    #[test]
    fn panel_rect_zero_window_does_not_panic() {
        let s = busy_state(2, 3);
        let (_, _, w, h) = panel_rect((0, 0), &s);
        assert!(w >= 0.0 && h >= 0.0);
    }

    #[test]
    fn cycle_focus_toggles() {
        let mut s = busy_state(1, 1);
        assert_eq!(s.focus, FocusedButton::Cancel);
        s.cycle_focus();
        assert_eq!(s.focus, FocusedButton::CloseAnyway);
        s.cycle_focus();
        assert_eq!(s.focus, FocusedButton::Cancel);
    }

    #[test]
    fn button_rects_are_inside_panel() {
        let panel = (100.0, 100.0, 640.0, 320.0);
        let bg = button_rects(panel);
        // Both buttons within panel.
        assert!(bg.cancel.0 >= panel.0);
        assert!(bg.cancel.0 + bg.cancel.2 <= panel.0 + panel.2);
        assert!(bg.cancel.1 + bg.cancel.3 <= panel.1 + panel.3);
        // Close anyway is right of Cancel.
        assert!(bg.close_anyway.0 > bg.cancel.0);
    }

    #[test]
    fn is_busy_mode_distinguishes_states() {
        let busy = busy_state(2, 3);
        let idle = busy_state(0, 3);
        assert!(busy.is_busy_mode());
        assert!(!idle.is_busy_mode());
    }
}

use crate::render::quad::QuadInstance;
use crate::render::tabs::RectInstance;
use crate::render::text_engine::TextEngine;

/// Build the per-frame rect list for the confirm-close overlay.
///
/// Order (DRAW ORDER — first paints first):
/// 0. Full-window backdrop dim.
/// 1. Panel body.
///    2–5. Four 2-px border edges (top, bottom, left, right).
/// 6.  Cancel button background.
/// 7.  Close-anyway button background.
pub fn build_confirm_close_rects(
    window_size: (u32, u32),
    state: &ConfirmCloseState,
    colors: &ConfirmCloseColors,
) -> Vec<RectInstance> {
    const BORDER_PX: f32 = 2.0;
    let panel = panel_rect(window_size, state);
    let (px, py, pw, ph) = panel;
    let window_w = window_size.0 as f32;
    let window_h = window_size.1 as f32;
    let bg = button_rects(panel);

    let cancel_color = if state.focus == FocusedButton::Cancel {
        colors.button_focus_bg
    } else {
        colors.button_idle_bg
    };
    let close_anyway_color = if state.focus == FocusedButton::CloseAnyway {
        colors.button_focus_bg
    } else {
        colors.button_idle_bg
    };

    vec![
        RectInstance::new(0.0, 0.0, window_w, window_h, colors.backdrop),
        RectInstance::new(px, py, pw, ph, colors.panel_bg),
        RectInstance::new(px, py, pw, BORDER_PX, colors.border_fg),
        RectInstance::new(px, py + ph - BORDER_PX, pw, BORDER_PX, colors.border_fg),
        RectInstance::new(px, py, BORDER_PX, ph, colors.border_fg),
        RectInstance::new(px + pw - BORDER_PX, py, BORDER_PX, ph, colors.border_fg),
        RectInstance::new(
            bg.cancel.0,
            bg.cancel.1,
            bg.cancel.2,
            bg.cancel.3,
            cancel_color,
        ),
        RectInstance::new(
            bg.close_anyway.0,
            bg.close_anyway.1,
            bg.close_anyway.2,
            bg.close_anyway.3,
            close_anyway_color,
        ),
    ]
}

/// Lines to render inside the panel, top-to-bottom. Visual layout matches
/// the spec §3.1; busy vs idle-multi-tab modes diverge after the title.
///
/// Cap busy-list rows at 8; overflow becomes `"  … and N more"`.
pub fn content_lines(state: &ConfirmCloseState) -> Vec<String> {
    const MAX_LIST_ROWS: usize = 8;
    let mut lines: Vec<String> = Vec::new();
    lines.push(title_text(state).to_string());
    lines.push(String::new()); // visual gap

    if state.is_busy_mode() {
        let n = state.busy_tabs.len();
        let word = if n == 1 { "session" } else { "sessions" };
        lines.push(format!("{n} {word} active:"));
        let shown = state.busy_tabs.len().min(MAX_LIST_ROWS);
        for info in state.busy_tabs.iter().take(shown) {
            lines.push(format!(
                "  • Tab {} ({})  — {}",
                info.tab_index, info.display_label, info.state_label
            ));
        }
        if state.busy_tabs.len() > MAX_LIST_ROWS {
            let extra = state.busy_tabs.len() - MAX_LIST_ROWS;
            lines.push(format!("  … and {extra} more"));
        }
    } else {
        match state.scope {
            ConfirmCloseScope::Window => {
                lines.push(format!("{} tabs are open. Close all?", state.tab_count));
            }
            ConfirmCloseScope::OtherTabs { .. } => {
                let n = state.tab_count;
                let word = if n == 1 { "tab" } else { "tabs" };
                lines.push(format!("Close {n} other {word}?"));
            }
            ConfirmCloseScope::SingleTab { .. } => {
                // Silent path in production (close_needs_confirmation = false
                // for an idle tab) — body kept terse for testability.
                lines.push("Close this tab?".to_string());
            }
        }
    }

    lines
}

/// Build glyph quads for the dialog text + button labels.
pub fn build_confirm_close_glyphs(
    window_size: (u32, u32),
    state: &ConfirmCloseState,
    text_engine: &mut TextEngine,
    colors: &ConfirmCloseColors,
) -> Vec<QuadInstance> {
    const INNER_PADDING_TOP: f32 = 20.0;
    const INNER_PADDING_X: f32 = 24.0;
    const BUTTON_BOTTOM_RESERVE: f32 = 68.0; // BUTTON_H + BOTTOM_GAP + slack

    let panel = panel_rect(window_size, state);
    let (px, py, pw, ph) = panel;
    let (cell_w, cell_h) = text_engine.cell_metrics();
    let cell_w_f = cell_w as f32;
    let cell_h_f = cell_h as f32;
    let inner_left = px + INNER_PADDING_X;
    let inner_right = px + pw - INNER_PADDING_X;
    let inner_width = (inner_right - inner_left).max(0.0);

    let mut glyphs: Vec<QuadInstance> = Vec::new();
    let lines = content_lines(state);
    let lines_top = py + INNER_PADDING_TOP;
    let lines_h_avail = (ph - INNER_PADDING_TOP - BUTTON_BOTTOM_RESERVE).max(0.0);
    let line_pitch = if !lines.is_empty() {
        lines_h_avail / lines.len() as f32
    } else {
        0.0
    };

    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let text_w = line.chars().count() as f32 * cell_w_f;
        // First line (title): centre. Other lines: left-align.
        let line_x = if i == 0 {
            inner_left + ((inner_width - text_w) / 2.0).max(0.0)
        } else {
            inner_left
        };
        let row_top = lines_top + i as f32 * line_pitch;
        let line_y = row_top + ((line_pitch - cell_h_f) / 2.0).max(0.0);
        let max_x = (inner_right).floor() as u32;
        crate::render::tabs::push_text_glyphs(
            &mut glyphs,
            text_engine,
            line,
            (line_x, line_y),
            cell_w_f,
            colors.text_fg,
            colors.panel_bg,
            max_x,
        );
    }

    // Button labels, centred within each button rect.
    let bg = button_rects(panel);
    for (label, geom, is_focused) in [
        ("Cancel", bg.cancel, state.focus == FocusedButton::Cancel),
        (
            confirm_button_label(state),
            bg.close_anyway,
            state.focus == FocusedButton::CloseAnyway,
        ),
    ] {
        let (bx, by, bw, bh) = geom;
        let text_w = label.chars().count() as f32 * cell_w_f;
        let label_x = bx + ((bw - text_w) / 2.0).max(0.0);
        let label_y = by + ((bh - cell_h_f) / 2.0).max(0.0);
        let max_x = (bx + bw).floor() as u32;
        let label_bg = if is_focused {
            colors.button_focus_bg
        } else {
            colors.button_idle_bg
        };
        crate::render::tabs::push_text_glyphs(
            &mut glyphs,
            text_engine,
            label,
            (label_x, label_y),
            cell_w_f,
            colors.button_text_fg,
            label_bg,
            max_x,
        );
    }

    glyphs
}

/// Hit-test a click position against the dialog. Returns `Some(button)` if
/// the click landed inside one of the two button rects; `None` if the click
/// missed (caller decides whether to dismiss based on inside-panel-vs-outside).
pub fn hit_test_buttons(
    window_size: (u32, u32),
    state: &ConfirmCloseState,
    click_pos: (f32, f32),
) -> Option<FocusedButton> {
    let bg = button_rects(panel_rect(window_size, state));
    let (x, y) = click_pos;
    if x >= bg.cancel.0
        && x <= bg.cancel.0 + bg.cancel.2
        && y >= bg.cancel.1
        && y <= bg.cancel.1 + bg.cancel.3
    {
        return Some(FocusedButton::Cancel);
    }
    if x >= bg.close_anyway.0
        && x <= bg.close_anyway.0 + bg.close_anyway.2
        && y >= bg.close_anyway.1
        && y <= bg.close_anyway.1 + bg.close_anyway.3
    {
        return Some(FocusedButton::CloseAnyway);
    }
    None
}

/// True when `pos` lies inside the panel rect (used to distinguish "click
/// missed the panel = dismiss" from "click inside panel but missed buttons
/// = consume + no action").
pub fn click_is_inside_panel(
    window_size: (u32, u32),
    state: &ConfirmCloseState,
    pos: (f32, f32),
) -> bool {
    let (px, py, pw, ph) = panel_rect(window_size, state);
    pos.0 >= px && pos.0 <= px + pw && pos.1 >= py && pos.1 <= py + ph
}

#[cfg(test)]
mod tests_content {
    use super::*;
    use crate::app::BusyTabInfo;

    fn one_busy() -> Vec<BusyTabInfo> {
        vec![BusyTabInfo {
            tab_index: 2,
            display_label: "claude".to_string(),
            state_label: "Waiting".to_string(),
        }]
    }

    #[test]
    fn content_lines_busy_mode_singular() {
        let state = ConfirmCloseState::new(one_busy(), 2);
        let lines = content_lines(&state);
        assert_eq!(lines[0], "Close vibeflow?");
        assert!(lines[2].contains("1 session active"), "got {:?}", lines[2]);
        assert!(lines[3].contains("Tab 2"));
        assert!(lines[3].contains("claude"));
        assert!(lines[3].contains("Waiting"));
    }

    #[test]
    fn content_lines_busy_mode_plural() {
        let busy: Vec<BusyTabInfo> = (1..=3)
            .map(|i| BusyTabInfo {
                tab_index: i,
                display_label: format!("p{i}"),
                state_label: "running".to_string(),
            })
            .collect();
        let state = ConfirmCloseState::new(busy, 3);
        let lines = content_lines(&state);
        assert!(lines[2].contains("3 sessions active"));
    }

    #[test]
    fn content_lines_busy_mode_caps_at_eight_with_overflow() {
        let busy: Vec<BusyTabInfo> = (1..=12)
            .map(|i| BusyTabInfo {
                tab_index: i,
                display_label: format!("p{i}"),
                state_label: "running".to_string(),
            })
            .collect();
        let state = ConfirmCloseState::new(busy, 12);
        let lines = content_lines(&state);
        // title + blank + summary + 8 list rows + overflow = 12 lines
        assert_eq!(lines.len(), 12);
        assert!(lines.last().unwrap().contains("… and 4 more"));
    }

    #[test]
    fn content_lines_idle_multi_tab_mode() {
        let state = ConfirmCloseState::new(Vec::new(), 3);
        let lines = content_lines(&state);
        assert_eq!(lines[0], "Close vibeflow?");
        assert_eq!(lines[2], "3 tabs are open. Close all?");
        assert_eq!(lines.len(), 3, "no list rows in idle-multi-tab mode");
    }

    #[test]
    fn hit_test_inside_cancel_rect_returns_cancel() {
        let state = ConfirmCloseState::new(one_busy(), 2);
        let panel = panel_rect((1920, 1080), &state);
        let bg = button_rects(panel);
        let click = (bg.cancel.0 + 5.0, bg.cancel.1 + 5.0);
        assert_eq!(
            hit_test_buttons((1920, 1080), &state, click),
            Some(FocusedButton::Cancel)
        );
    }

    #[test]
    fn hit_test_inside_close_anyway_rect_returns_close_anyway() {
        let state = ConfirmCloseState::new(one_busy(), 2);
        let panel = panel_rect((1920, 1080), &state);
        let bg = button_rects(panel);
        let click = (bg.close_anyway.0 + 5.0, bg.close_anyway.1 + 5.0);
        assert_eq!(
            hit_test_buttons((1920, 1080), &state, click),
            Some(FocusedButton::CloseAnyway)
        );
    }

    #[test]
    fn hit_test_outside_buttons_returns_none() {
        let state = ConfirmCloseState::new(one_busy(), 2);
        assert_eq!(hit_test_buttons((1920, 1080), &state, (10.0, 10.0)), None);
    }

    #[test]
    fn click_is_inside_panel_rejects_corner_pixel() {
        let state = ConfirmCloseState::new(one_busy(), 2);
        assert!(!click_is_inside_panel((1920, 1080), &state, (0.0, 0.0)));
    }

    #[test]
    fn click_is_inside_panel_accepts_centre() {
        let state = ConfirmCloseState::new(one_busy(), 2);
        assert!(click_is_inside_panel((1920, 1080), &state, (960.0, 540.0)));
    }

    // -------- v0.1.3 per-tab close amendment --------

    #[test]
    fn new_defaults_scope_to_window() {
        let state = ConfirmCloseState::new(Vec::new(), 2);
        assert!(matches!(state.scope, ConfirmCloseScope::Window));
    }

    #[test]
    fn with_scope_records_single_tab() {
        let state = ConfirmCloseState::with_scope(
            Vec::new(),
            1,
            ConfirmCloseScope::SingleTab { tab_index: 2 },
        );
        match state.scope {
            ConfirmCloseScope::SingleTab { tab_index } => assert_eq!(tab_index, 2),
            _ => panic!("expected SingleTab scope"),
        }
    }

    #[test]
    fn with_scope_records_other_tabs() {
        let state = ConfirmCloseState::with_scope(
            Vec::new(),
            4,
            ConfirmCloseScope::OtherTabs { keep_tab_index: 1 },
        );
        match state.scope {
            ConfirmCloseScope::OtherTabs { keep_tab_index } => assert_eq!(keep_tab_index, 1),
            _ => panic!("expected OtherTabs scope"),
        }
    }

    #[test]
    fn title_text_per_scope() {
        let win = ConfirmCloseState::new(Vec::new(), 2);
        let one = ConfirmCloseState::with_scope(
            Vec::new(),
            1,
            ConfirmCloseScope::SingleTab { tab_index: 0 },
        );
        let oth = ConfirmCloseState::with_scope(
            Vec::new(),
            3,
            ConfirmCloseScope::OtherTabs { keep_tab_index: 0 },
        );
        assert_eq!(title_text(&win), "Close vibeflow?");
        assert_eq!(title_text(&one), "Close this tab?");
        assert_eq!(title_text(&oth), "Close other tabs?");
    }

    #[test]
    fn confirm_button_label_per_scope() {
        let win = ConfirmCloseState::new(Vec::new(), 2);
        let one = ConfirmCloseState::with_scope(
            Vec::new(),
            1,
            ConfirmCloseScope::SingleTab { tab_index: 0 },
        );
        let oth = ConfirmCloseState::with_scope(
            Vec::new(),
            3,
            ConfirmCloseScope::OtherTabs { keep_tab_index: 0 },
        );
        assert_eq!(confirm_button_label(&win), "Close anyway");
        assert_eq!(confirm_button_label(&one), "Close tab");
        assert_eq!(confirm_button_label(&oth), "Close other tabs");
    }

    #[test]
    fn content_lines_single_tab_busy_shows_title_and_entry() {
        let state = ConfirmCloseState::with_scope(
            one_busy(),
            1,
            ConfirmCloseScope::SingleTab { tab_index: 1 },
        );
        let lines = content_lines(&state);
        assert_eq!(lines[0], "Close this tab?");
        assert!(lines[3].contains("Tab 2"));
        assert!(lines[3].contains("claude"));
    }

    #[test]
    fn content_lines_other_tabs_idle_shows_count() {
        let state = ConfirmCloseState::with_scope(
            Vec::new(),
            2,
            ConfirmCloseScope::OtherTabs { keep_tab_index: 1 },
        );
        let lines = content_lines(&state);
        assert_eq!(lines[0], "Close other tabs?");
        assert_eq!(lines[2], "Close 2 other tabs?");
    }

    #[test]
    fn content_lines_other_tabs_singular_word() {
        let state = ConfirmCloseState::with_scope(
            Vec::new(),
            1,
            ConfirmCloseScope::OtherTabs { keep_tab_index: 1 },
        );
        let lines = content_lines(&state);
        assert_eq!(lines[2], "Close 1 other tab?");
    }
}
