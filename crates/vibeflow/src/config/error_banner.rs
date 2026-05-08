//! In-window banner state for "N config keys ignored" errors. Pure logic;
//! the renderer reads `ErrorBannerState` and emits one rect range + one
//! glyph range per frame when the banner is visible and not dismissed.
