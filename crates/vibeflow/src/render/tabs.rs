//! Tab-bar rendering. Three pieces:
//!  * [`TabBarLayout`] — pure logic, computes per-tab rectangles + button hit zones.
//!  * `TabBarPipeline` — wgpu pipeline-state for solid-color rectangles
//!    (tab backgrounds, indicator stripes, separators, button bodies).
//!  * `TabBarRenderer` — glue that builds the per-frame instance lists from
//!    [`crate::app::App`] state + tracker states, including the Notice
//!    indicator pulse animation on `Waiting` tabs.
