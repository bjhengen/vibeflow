//! `serde::Deserialize`-derivable schema types matching the on-disk TOML
//! layout. All fields are `Option<T>` for partial-parse tolerance — missing
//! keys map to `None` here and are filled with defaults by `Config::load`.

use serde::Deserialize;
use std::collections::HashMap;

/// Top-level TOML structure. Every field is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigFile {
    pub shortcuts: Option<ShortcutsSection>,
    pub colors: Option<ColorsSection>,
    pub cursor: Option<CursorSection>,
    pub fonts: Option<FontsSection>,
    pub clipboard: Option<ClipboardSection>,
}

/// `[shortcuts]` table. Each known action key (e.g. `new_tab`, `copy`) maps
/// to a list of key-spec strings (e.g. `["ctrl+shift+t", "super+t"]`).
///
/// The `extra` field catches unknown action keys so we can emit warnings
/// without aborting the whole parse.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ShortcutsSection {
    pub new_tab: Option<Vec<String>>,
    pub close_tab: Option<Vec<String>>,
    pub next_tab: Option<Vec<String>>,
    pub prev_tab: Option<Vec<String>>,
    pub restart_tab: Option<Vec<String>>,
    pub copy: Option<Vec<String>>,
    pub paste: Option<Vec<String>>,
    pub rename_tab: Option<Vec<String>>,
    /// Unknown action keys land here so `mod.rs::load` can warn about them.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// `[colors]` table. RGBA hex strings like `"#6699FF66"`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColorsSection {
    pub selection: Option<String>,
    pub indicator_active: Option<String>,
    pub indicator_working: Option<String>,
    pub indicator_waiting: Option<String>,
    pub indicator_inactive: Option<String>,
}

/// `[cursor]` table. `blink_ms = 0` disables blinking.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CursorSection {
    pub blink_ms: Option<u64>,
}

/// `[fonts]` table. Ordered priority list; earlier entries take precedence.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontsSection {
    pub priority: Option<Vec<String>>,
}

/// `[clipboard]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClipboardSection {
    pub primary: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ConfigFile {
        toml::from_str(s).expect("valid TOML")
    }

    #[test]
    fn empty_file_parses_to_all_none() {
        let cf = parse("");
        assert!(cf.shortcuts.is_none());
        assert!(cf.colors.is_none());
        assert!(cf.cursor.is_none());
        assert!(cf.fonts.is_none());
        assert!(cf.clipboard.is_none());
    }

    #[test]
    fn full_file_parses() {
        let s = r##"
            [shortcuts]
            new_tab = ["ctrl+shift+t", "super+t"]
            copy = ["ctrl+shift+c"]

            [colors]
            selection = "#6699ff66"
            indicator_active = "#22cc66ff"

            [cursor]
            blink_ms = 250

            [fonts]
            priority = ["JetBrains Mono", "Noto Color Emoji"]

            [clipboard]
            primary = true
        "##;
        let cf = parse(s);
        let shortcuts = cf.shortcuts.expect("shortcuts");
        assert_eq!(
            shortcuts.new_tab.as_deref(),
            Some(&["ctrl+shift+t".to_string(), "super+t".to_string()][..])
        );
        assert_eq!(
            shortcuts.copy.as_deref(),
            Some(&["ctrl+shift+c".to_string()][..])
        );
        let colors = cf.colors.expect("colors");
        assert_eq!(colors.selection.as_deref(), Some("#6699ff66"));
        assert_eq!(colors.indicator_active.as_deref(), Some("#22cc66ff"));
        assert_eq!(cf.cursor.expect("cursor").blink_ms, Some(250));
        assert_eq!(
            cf.fonts.expect("fonts").priority.as_deref(),
            Some(&["JetBrains Mono".to_string(), "Noto Color Emoji".to_string()][..])
        );
        assert_eq!(cf.clipboard.expect("clipboard").primary, Some(true));
    }

    #[test]
    fn partial_section_only_fills_named_fields() {
        let s = r##"
            [colors]
            selection = "#ff0000ff"
        "##;
        let cf = parse(s);
        let colors = cf.colors.expect("colors");
        assert_eq!(colors.selection.as_deref(), Some("#ff0000ff"));
        assert_eq!(colors.indicator_active, None);
    }

    #[test]
    fn unknown_top_level_key_fails_parse() {
        // `deny_unknown_fields` rejects typos at the top level.
        let s = r##"
            [colros]
            selection = "#000000ff"
        "##;
        let r: Result<ConfigFile, _> = toml::from_str(s);
        assert!(r.is_err(), "expected parse error for unknown top-level key");
    }

    #[test]
    fn unknown_shortcut_action_lands_in_extra() {
        let s = r##"
            [shortcuts]
            new_tab = ["ctrl+shift+t"]
            launch_rocket = ["ctrl+r"]
        "##;
        let cf = parse(s);
        let shortcuts = cf.shortcuts.expect("shortcuts");
        assert!(shortcuts.extra.contains_key("launch_rocket"));
        assert_eq!(shortcuts.new_tab.unwrap().len(), 1);
    }

    #[test]
    fn unknown_colors_field_fails_parse() {
        // ColorsSection has deny_unknown_fields — typos rejected.
        let s = r##"
            [colors]
            selectoin = "#ff0000ff"
        "##;
        let r: Result<ConfigFile, _> = toml::from_str(s);
        assert!(r.is_err());
    }
}
