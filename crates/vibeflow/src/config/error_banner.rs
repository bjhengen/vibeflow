//! In-window banner state for "N config keys ignored" errors. Pure logic;
//! the renderer reads `ErrorBannerState` and emits one rect range + one
//! glyph range per frame when the banner is visible and not dismissed.

use crate::config::ConfigError;

/// State for the config-error banner. Visible iff `!errors.is_empty()` and
/// `!dismissed`.
#[derive(Debug, Clone, Default)]
pub struct ErrorBannerState {
    pub errors: Vec<ConfigError>,
    pub dismissed: bool,
}

impl ErrorBannerState {
    #[must_use]
    pub fn new(errors: Vec<ConfigError>) -> Self {
        Self {
            errors,
            dismissed: false,
        }
    }

    /// True if the banner should currently be drawn.
    #[must_use]
    pub fn visible(&self) -> bool {
        !self.errors.is_empty() && !self.dismissed
    }

    /// User pressed Esc — hide the banner. Stays hidden until the next
    /// `update()` call replaces the errors.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Replace the error list. Resets `dismissed` so a new error reappears
    /// even if the user had dismissed the previous one.
    pub fn update(&mut self, errors: Vec<ConfigError>) {
        self.errors = errors;
        self.dismissed = false;
    }

    /// Single-line text to render in the banner. Includes the count and the
    /// short-form of the first error; appends `… (N more)` when `errors.len() > 1`.
    #[must_use]
    pub fn display_text(&self) -> String {
        let n = self.errors.len();
        if n == 0 {
            return String::new();
        }
        let first = self.errors[0].short();
        let suffix = if n > 1 {
            format!(" … ({} more)", n - 1)
        } else {
            String::new()
        };
        format!(
            "⚠ {n} config key{} ignored: {first}{suffix} — Esc to dismiss",
            if n == 1 { "" } else { "s" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_color(key: &str, value: &str) -> ConfigError {
        ConfigError::InvalidColor {
            key: key.to_string(),
            value: value.to_string(),
            msg: "expected 8 hex digits".to_string(),
        }
    }

    #[test]
    fn empty_banner_is_not_visible() {
        let b = ErrorBannerState::default();
        assert!(!b.visible());
    }

    #[test]
    fn banner_with_errors_is_visible() {
        let b = ErrorBannerState::new(vec![err_color("selection", "bad")]);
        assert!(b.visible());
    }

    #[test]
    fn dismiss_hides_banner() {
        let mut b = ErrorBannerState::new(vec![err_color("selection", "bad")]);
        b.dismiss();
        assert!(!b.visible());
    }

    #[test]
    fn update_resets_dismissed() {
        let mut b = ErrorBannerState::new(vec![err_color("selection", "bad")]);
        b.dismiss();
        assert!(!b.visible());
        b.update(vec![err_color("indicator_active", "bad2")]);
        assert!(b.visible());
    }

    #[test]
    fn update_to_empty_clears_visibility() {
        let mut b = ErrorBannerState::new(vec![err_color("selection", "bad")]);
        b.update(vec![]);
        assert!(!b.visible());
    }

    #[test]
    fn display_text_singular() {
        let b = ErrorBannerState::new(vec![err_color("selection", "xyz")]);
        let t = b.display_text();
        assert!(t.contains("1 config key ignored"));
        assert!(t.contains("selection"));
        assert!(t.contains("xyz"));
    }

    #[test]
    fn display_text_plural_with_count_suffix() {
        let b = ErrorBannerState::new(vec![
            err_color("selection", "x"),
            err_color("indicator_active", "y"),
            err_color("indicator_working", "z"),
        ]);
        let t = b.display_text();
        assert!(t.contains("3 config keys ignored"));
        assert!(t.contains("selection"));
        assert!(t.contains("(2 more)"));
    }

    #[test]
    fn display_text_empty_is_empty() {
        let b = ErrorBannerState::default();
        assert_eq!(b.display_text(), "");
    }
}
