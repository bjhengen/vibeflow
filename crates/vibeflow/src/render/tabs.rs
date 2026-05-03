//! Tab-bar rendering. Three pieces:
//!  * [`TabBarLayout`] — pure logic, computes per-tab rectangles + button hit zones.
//!  * `TabBarPipeline` — wgpu pipeline-state for solid-color rectangles
//!    (tab backgrounds, indicator stripes, separators, button bodies).
//!  * `TabBarRenderer` — glue that builds the per-frame instance lists from
//!    [`crate::app::App`] state + tracker states, including the Notice
//!    indicator pulse animation on `Waiting` tabs.

/// Stage-6 default tab-bar height in pixels, expressed as (line_height × 2 + padding).
/// Computed at runtime from the atlas's cell pitch.
#[must_use]
pub fn tab_bar_height_px(cell_h_px: u32) -> u32 {
    cell_h_px * 2 + 8
}

/// Pixel width of the `+` (new tab) button at the right end of the bar.
pub const NEW_TAB_BUTTON_WIDTH_PX: u32 = 32;

/// Pixel width of the per-tab `×` close button.
pub const CLOSE_BUTTON_WIDTH_PX: u32 = 20;

/// Maximum pixel width any single tab is allowed to stretch to.
pub const MAX_TAB_WIDTH_PX: u32 = 250;

/// Minimum pixel width any tab is shown with (below this, the close button overlaps the title).
pub const MIN_TAB_WIDTH_PX: u32 = 80;

/// Layout result. Owns no GPU state — purely numeric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBarLayout {
    pub bar_height_px: u32,
    pub tabs: Vec<TabRect>,
    pub new_tab_button: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[must_use]
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRect {
    pub idx: usize,
    pub body: Rect,
    pub close_button: Rect,
}

/// What a click at a given (px, py) hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarHit {
    /// Click on the tab body (anywhere except the close button) — should focus.
    TabBody(usize),
    /// Click on a tab's `×` close button — should close that tab.
    TabClose(usize),
    /// Click on the `+` button — should spawn a new tab.
    NewTab,
    /// Click landed on a separator or empty space — no action.
    None,
}

impl TabBarLayout {
    /// Compute layout for `tab_count` tabs in a `window_width_px`-wide window.
    /// `cell_h_px` comes from the atlas — this is what bounds the bar height.
    #[must_use]
    pub fn compute(window_width_px: u32, cell_h_px: u32, tab_count: usize) -> Self {
        let bar_height_px = tab_bar_height_px(cell_h_px);
        let new_tab_button = Rect {
            x: window_width_px.saturating_sub(NEW_TAB_BUTTON_WIDTH_PX),
            y: 0,
            w: NEW_TAB_BUTTON_WIDTH_PX,
            h: bar_height_px,
        };

        if tab_count == 0 {
            return Self {
                bar_height_px,
                tabs: Vec::new(),
                new_tab_button,
            };
        }

        let avail_width = window_width_px.saturating_sub(NEW_TAB_BUTTON_WIDTH_PX);
        let raw_tab_w = avail_width / tab_count as u32;
        let tab_w = raw_tab_w.clamp(MIN_TAB_WIDTH_PX, MAX_TAB_WIDTH_PX);

        let mut tabs = Vec::with_capacity(tab_count);
        for idx in 0..tab_count {
            let x = (idx as u32) * tab_w;
            let body = Rect {
                x,
                y: 0,
                w: tab_w,
                h: bar_height_px,
            };
            // Close button at the right edge of the tab's body.
            let close_button = Rect {
                x: x + tab_w.saturating_sub(CLOSE_BUTTON_WIDTH_PX + 4),
                y: bar_height_px / 2 - CLOSE_BUTTON_WIDTH_PX / 2,
                w: CLOSE_BUTTON_WIDTH_PX,
                h: CLOSE_BUTTON_WIDTH_PX,
            };
            tabs.push(TabRect {
                idx,
                body,
                close_button,
            });
        }

        Self {
            bar_height_px,
            tabs,
            new_tab_button,
        }
    }

    /// Hit-test a click at (px, py). Order: close button > tab body > new-tab > none.
    #[must_use]
    pub fn hit_test(&self, px: u32, py: u32) -> TabBarHit {
        if py >= self.bar_height_px {
            return TabBarHit::None; // click below the tab bar
        }
        if self.new_tab_button.contains(px, py) {
            return TabBarHit::NewTab;
        }
        for tab in &self.tabs {
            if tab.close_button.contains(px, py) {
                return TabBarHit::TabClose(tab.idx);
            }
            if tab.body.contains(px, py) {
                return TabBarHit::TabBody(tab.idx);
            }
        }
        TabBarHit::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_height_is_double_cell_plus_padding() {
        assert_eq!(tab_bar_height_px(20), 48);
        assert_eq!(tab_bar_height_px(22), 52);
    }

    #[test]
    fn compute_with_zero_tabs_returns_empty_tabs_and_button_at_right() {
        let layout = TabBarLayout::compute(960, 22, 0);
        assert!(layout.tabs.is_empty());
        assert_eq!(layout.new_tab_button.x, 960 - NEW_TAB_BUTTON_WIDTH_PX);
        assert_eq!(layout.new_tab_button.w, NEW_TAB_BUTTON_WIDTH_PX);
    }

    #[test]
    fn compute_one_tab_takes_full_available_width_clamped_to_max() {
        // 960 - 32 (new tab btn) = 928, which exceeds MAX_TAB_WIDTH_PX = 250.
        let layout = TabBarLayout::compute(960, 22, 1);
        assert_eq!(layout.tabs.len(), 1);
        assert_eq!(layout.tabs[0].body.x, 0);
        assert_eq!(layout.tabs[0].body.w, MAX_TAB_WIDTH_PX);
    }

    #[test]
    fn compute_many_tabs_packs_to_min_width() {
        // 14 tabs in 960 px: 928 / 14 = 66 px per tab, below MIN = 80.
        let layout = TabBarLayout::compute(960, 22, 14);
        assert_eq!(layout.tabs.len(), 14);
        for (i, tab) in layout.tabs.iter().enumerate() {
            assert_eq!(tab.body.w, MIN_TAB_WIDTH_PX);
            assert_eq!(tab.body.x, (i as u32) * MIN_TAB_WIDTH_PX);
        }
    }

    #[test]
    fn hit_test_below_bar_returns_none() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // y past the bar height
        assert_eq!(layout.hit_test(100, 100), TabBarHit::None);
    }

    #[test]
    fn hit_test_on_tab_body_returns_tab_body() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // First tab spans x=0..250 (clamped to MAX). Click at (50, 10) is inside.
        assert_eq!(layout.hit_test(50, 10), TabBarHit::TabBody(0));
    }

    #[test]
    fn hit_test_on_close_button_returns_tab_close() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // First tab's close button is near the right edge of the tab body.
        let close = layout.tabs[0].close_button;
        assert_eq!(
            layout.hit_test(close.x + 1, close.y + 1),
            TabBarHit::TabClose(0)
        );
    }

    #[test]
    fn hit_test_on_new_tab_button_returns_new_tab() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // The + button is at x=960-32=928, y=0.
        assert_eq!(layout.hit_test(940, 10), TabBarHit::NewTab);
    }

    #[test]
    fn hit_test_in_gap_between_tabs_returns_none_or_body() {
        // Tabs are contiguous (no visual gap in Stage 6); every x within
        // [0, total_tabs_width) is some tab. Adding a separator is a Stage 9
        // visual polish item.
        let layout = TabBarLayout::compute(960, 22, 4);
        // 4 tabs, 928 / 4 = 232 px each (< MAX 250). x=232 is the start of tab 1.
        assert_eq!(layout.hit_test(232, 10), TabBarHit::TabBody(1));
    }
}
