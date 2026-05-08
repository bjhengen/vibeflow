//! TOML configuration: schema types, parsing, hot-reload, and the
//! `AppUserEvent` enum delivered via `EventLoopProxy::send_event` from the
//! file-watcher thread to `WindowApp::user_event` on the main thread.

pub mod error_banner;
pub mod schema;
pub mod watcher;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use winit::keyboard::ModifiersState;

use crate::keymap::Shortcut;

/// Resolved configuration after defaults are applied. All fields are concrete
/// (no `Option`s) — anything missing in the file is filled here.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub shortcuts: ShortcutBindings,
    pub colors: Colors,
    pub cursor: CursorConfig,
    pub fonts: FontsConfig,
    pub clipboard: ClipboardConfig,
}

/// Action → list of (modifiers, key) pairs that trigger it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShortcutBindings {
    pub bindings: HashMap<Shortcut, Vec<KeyChord>>,
}

/// One concrete chord: modifier mask + key matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub modifiers: ModifiersState,
    pub key: KeyMatch,
}

/// What logical key the chord matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMatch {
    /// `Key::Character(c)` where `c.as_str().to_ascii_lowercase() == ch`.
    Char(char),
    /// `Key::Named(NamedKey::Tab)`.
    Tab,
    /// `Key::Named(NamedKey::F1)` through `F12`. Stored as 1..=12.
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub selection: [f32; 4],
    pub indicator_active: [f32; 4],
    pub indicator_working: [f32; 4],
    pub indicator_waiting: [f32; 4],
    pub indicator_inactive: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorConfig {
    pub blink_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontsConfig {
    pub priority: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipboardConfig {
    pub primary: bool,
}

/// One thing that went wrong during config load.
#[derive(Debug, Clone)]
pub enum ConfigError {
    /// TOML syntax error from the `toml` crate.
    Syntax {
        line: usize,
        col: usize,
        msg: String,
    },
    /// Filesystem error (permission denied, missing dir, etc.).
    IoError(String),
    /// `#RRGGBBAA` parse failure.
    InvalidColor {
        key: String,
        value: String,
        msg: String,
    },
    /// Shortcut spec parse failure (`"ctrl+shift+t"`).
    InvalidShortcut {
        action: String,
        value: String,
        msg: String,
    },
    /// Unknown shortcut action (lands in `ShortcutsSection.extra`).
    UnknownAction(String),
    /// Catch-all for "expected u64 but got string".
    InvalidValue {
        key: String,
        expected: String,
        got: String,
    },
}

impl ConfigError {
    /// Short-form text for the error banner.
    #[must_use]
    pub fn short(&self) -> String {
        match self {
            Self::Syntax { line, col, msg } => format!("syntax error at L{line}:C{col}: {msg}"),
            Self::IoError(msg) => format!("I/O: {msg}"),
            Self::InvalidColor { key, value, msg } => format!("colors.{key} = \"{value}\": {msg}"),
            Self::InvalidShortcut { action, value, msg } => {
                format!("shortcuts.{action} entry \"{value}\": {msg}")
            }
            Self::UnknownAction(name) => format!("shortcuts.{name}: unknown action"),
            Self::InvalidValue { key, expected, got } => {
                format!("{key}: expected {expected}, got {got}")
            }
        }
    }
}

impl Config {
    /// Built-in defaults — what vibeflow does when no config file exists.
    #[must_use]
    pub fn default_values() -> Self {
        Self {
            shortcuts: default_shortcuts(),
            colors: Colors {
                selection: rgba(0x66, 0x99, 0xFF, 0x66),
                indicator_active: rgba(0x22, 0xCC, 0x66, 0xFF),
                indicator_working: rgba(0x33, 0x99, 0xFF, 0xFF),
                indicator_waiting: rgba(0xFF, 0xAA, 0x00, 0xFF),
                indicator_inactive: rgba(0x88, 0x88, 0x88, 0xFF),
            },
            cursor: CursorConfig { blink_ms: 500 },
            fonts: FontsConfig {
                priority: vec![
                    "JetBrains Mono".to_string(),
                    "Noto Color Emoji".to_string(),
                    "DejaVu Sans Mono".to_string(),
                ],
            },
            clipboard: ClipboardConfig { primary: true },
        }
    }

    /// Load from `path`. Returns `(Config, Vec<ConfigError>)` — the resolved
    /// config (with defaults filling any missing keys) plus any errors
    /// encountered while parsing. Per-key tolerance: bad keys fall back to
    /// defaults; valid keys still apply.
    ///
    /// File missing → defaults + empty error vec.
    /// File present but unreadable → defaults + one IoError.
    /// File present, syntax error → defaults + one Syntax error.
    /// File present, valid TOML, some keys bad → defaults filled per-key + per-error.
    pub fn load(path: &Path) -> (Self, Vec<ConfigError>) {
        let mut defaults = Self::default_values();
        let mut errors = Vec::new();

        let bytes = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Missing file: silent defaults.
                return (defaults, errors);
            }
            Err(e) => {
                errors.push(ConfigError::IoError(format!("{}: {}", path.display(), e)));
                return (defaults, errors);
            }
        };

        let file: schema::ConfigFile = match toml::from_str(&bytes) {
            Ok(f) => f,
            Err(e) => {
                let span = e.span().unwrap_or(0..0);
                let (line, col) = byte_offset_to_line_col(&bytes, span.start);
                errors.push(ConfigError::Syntax {
                    line,
                    col,
                    msg: e.message().to_string(),
                });
                return (defaults, errors);
            }
        };

        // Apply each section per-key with tolerance.
        if let Some(s) = file.shortcuts {
            apply_shortcuts(&mut defaults.shortcuts, s, &mut errors);
        }
        if let Some(c) = file.colors {
            apply_colors(&mut defaults.colors, c, &mut errors);
        }
        if let Some(c) = file.cursor {
            if let Some(ms) = c.blink_ms {
                defaults.cursor.blink_ms = ms;
            }
        }
        if let Some(f) = file.fonts {
            if let Some(p) = f.priority {
                defaults.fonts.priority = p;
            }
        }
        if let Some(cb) = file.clipboard {
            if let Some(p) = cb.primary {
                defaults.clipboard.primary = p;
            }
        }

        (defaults, errors)
    }
}

/// XDG-compliant path: `$XDG_CONFIG_HOME/vibeflow/config.toml` or
/// `~/.config/vibeflow/config.toml` on Linux; `~/Library/Application
/// Support/vibeflow/config.toml` on macOS.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("vibeflow").join("config.toml"))
}

// -- helpers below ----------------------------------------------------------

fn byte_offset_to_line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> [f32; 4] {
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

/// Parse `#RRGGBBAA` (8 hex digits, leading `#`). 6-digit forms are rejected
/// (alpha is required). Returns `Err(msg)` on any failure.
pub fn parse_color(s: &str) -> Result<[f32; 4], String> {
    let body = s
        .strip_prefix('#')
        .ok_or_else(|| format!("expected leading '#', got \"{s}\""))?;
    if body.len() != 8 {
        return Err(format!(
            "expected 8 hex digits after '#' (RRGGBBAA), got {} chars",
            body.len()
        ));
    }
    let mut parts = [0u8; 4];
    for i in 0..4 {
        parts[i] = u8::from_str_radix(&body[i * 2..i * 2 + 2], 16).map_err(|_| {
            format!(
                "invalid hex pair \"{}\" in \"{s}\"",
                &body[i * 2..i * 2 + 2]
            )
        })?;
    }
    Ok(rgba(parts[0], parts[1], parts[2], parts[3]))
}

/// Parse a shortcut spec string like `"ctrl+shift+t"` or `"super+tab"` or
/// `"f2"`. Modifiers are case-insensitive; key tokens are case-insensitive.
pub fn parse_shortcut(s: &str) -> Result<KeyChord, String> {
    let mut mods = ModifiersState::empty();
    let mut key: Option<KeyMatch> = None;
    for raw in s.split('+') {
        let tok = raw.trim().to_ascii_lowercase();
        match tok.as_str() {
            "ctrl" | "control" => mods |= ModifiersState::CONTROL,
            "shift" => mods |= ModifiersState::SHIFT,
            "alt" => mods |= ModifiersState::ALT,
            "super" | "cmd" | "meta" => mods |= ModifiersState::SUPER,
            "tab" => {
                if key.is_some() {
                    return Err(format!("multiple key tokens in \"{s}\""));
                }
                key = Some(KeyMatch::Tab);
            }
            other if other.starts_with('f') && other.len() <= 3 => {
                let n: u8 = other[1..]
                    .parse()
                    .map_err(|_| format!("unknown token \"{other}\""))?;
                if !(1..=12).contains(&n) {
                    return Err(format!("function key f{n} out of range (f1..f12)"));
                }
                if key.is_some() {
                    return Err(format!("multiple key tokens in \"{s}\""));
                }
                key = Some(KeyMatch::Function(n));
            }
            other if other.len() == 1 && other.is_ascii() => {
                if key.is_some() {
                    return Err(format!("multiple key tokens in \"{s}\""));
                }
                key = Some(KeyMatch::Char(other.chars().next().unwrap()));
            }
            other => return Err(format!("unknown token \"{other}\" in \"{s}\"")),
        }
    }
    let key = key.ok_or_else(|| format!("no key token in \"{s}\""))?;
    Ok(KeyChord {
        modifiers: mods,
        key,
    })
}

fn default_shortcuts() -> ShortcutBindings {
    use Shortcut::*;
    let entries: &[(Shortcut, &[&str])] = &[
        (NewTab, &["ctrl+shift+t", "super+t"]),
        (CloseTab, &["ctrl+shift+w", "super+w"]),
        (NextTab, &["ctrl+tab", "super+tab"]),
        (PrevTab, &["ctrl+shift+tab", "super+shift+tab"]),
        (RestartTab, &["ctrl+shift+r", "super+r"]),
        (Copy, &["ctrl+shift+c", "super+c"]),
        (Paste, &["ctrl+shift+v", "super+v"]),
        (RenameTab, &["ctrl+shift+e", "f2"]),
    ];
    let mut bindings = HashMap::new();
    for (action, specs) in entries {
        let chords: Vec<_> = specs
            .iter()
            .map(|s| parse_shortcut(s).expect("default shortcut parses"))
            .collect();
        bindings.insert(*action, chords);
    }
    ShortcutBindings { bindings }
}

fn apply_shortcuts(
    out: &mut ShortcutBindings,
    section: schema::ShortcutsSection,
    errors: &mut Vec<ConfigError>,
) {
    let mut apply = |action: Shortcut, action_name: &str, specs: Option<Vec<String>>| {
        let Some(specs) = specs else { return };
        // Empty list `[]` is an explicit "disable this action" — apply it.
        // A non-empty list whose entries all fail to parse keeps the default
        // (so a typo doesn't silently disable the action).
        let user_intent_disable = specs.is_empty();
        let mut chords = Vec::new();
        for spec in specs {
            match parse_shortcut(&spec) {
                Ok(c) => chords.push(c),
                Err(msg) => errors.push(ConfigError::InvalidShortcut {
                    action: action_name.to_string(),
                    value: spec,
                    msg,
                }),
            }
        }
        if user_intent_disable || !chords.is_empty() {
            out.bindings.insert(action, chords);
        }
        // else: every spec failed to parse → keep the default binding.
    };
    apply(Shortcut::NewTab, "new_tab", section.new_tab);
    apply(Shortcut::CloseTab, "close_tab", section.close_tab);
    apply(Shortcut::NextTab, "next_tab", section.next_tab);
    apply(Shortcut::PrevTab, "prev_tab", section.prev_tab);
    apply(Shortcut::RestartTab, "restart_tab", section.restart_tab);
    apply(Shortcut::Copy, "copy", section.copy);
    apply(Shortcut::Paste, "paste", section.paste);
    apply(Shortcut::RenameTab, "rename_tab", section.rename_tab);
    for unknown in section.extra.keys() {
        errors.push(ConfigError::UnknownAction(unknown.clone()));
    }
}

fn apply_colors(out: &mut Colors, section: schema::ColorsSection, errors: &mut Vec<ConfigError>) {
    let mut apply = |name: &str, slot: &mut [f32; 4], val: Option<String>| {
        let Some(val) = val else { return };
        match parse_color(&val) {
            Ok(c) => *slot = c,
            Err(msg) => errors.push(ConfigError::InvalidColor {
                key: name.to_string(),
                value: val,
                msg,
            }),
        }
    };
    apply("selection", &mut out.selection, section.selection);
    apply(
        "indicator_active",
        &mut out.indicator_active,
        section.indicator_active,
    );
    apply(
        "indicator_working",
        &mut out.indicator_working,
        section.indicator_working,
    );
    apply(
        "indicator_waiting",
        &mut out.indicator_waiting,
        section.indicator_waiting,
    );
    apply(
        "indicator_inactive",
        &mut out.indicator_inactive,
        section.indicator_inactive,
    );
}

/// Events delivered to the main thread via `EventLoopProxy::send_event`. The
/// only sender is the file-watcher thread (Task 6).
#[derive(Debug, Clone)]
pub enum AppUserEvent {
    /// New `Config` (with all defaults applied) plus any errors encountered
    /// during parse. `errors.is_empty()` means a clean reload.
    ConfigReloaded {
        config: Config,
        errors: Vec<ConfigError>,
    },
    /// One-off error not tied to a successful reload (file removed at
    /// runtime, IO error). Banner shows it; current `Config` is retained.
    ConfigError(ConfigError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn approx_eq(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter()
            .zip(b.iter())
            .all(|(x, y)| (x - y).abs() < 1.0 / 255.0 + f32::EPSILON)
    }

    // -- color parser ------------------------------------------------------

    #[test]
    fn parse_color_8_digit_hex() {
        let c = parse_color("#6699FF66").unwrap();
        assert!(approx_eq(c, [0.4, 0.6, 1.0, 0.4]));
    }

    #[test]
    fn parse_color_lowercase_hex() {
        let c = parse_color("#6699ff66").unwrap();
        assert!(approx_eq(c, [0.4, 0.6, 1.0, 0.4]));
    }

    #[test]
    fn parse_color_rejects_6_digit() {
        let r = parse_color("#6699FF");
        assert!(r.is_err());
    }

    #[test]
    fn parse_color_rejects_3_digit() {
        let r = parse_color("#69F");
        assert!(r.is_err());
    }

    #[test]
    fn parse_color_rejects_missing_hash() {
        let r = parse_color("6699FF66");
        assert!(r.is_err());
    }

    #[test]
    fn parse_color_alpha_correctness() {
        let c = parse_color("#000000FF").unwrap();
        assert!(approx_eq(c, [0.0, 0.0, 0.0, 1.0]));
        let c = parse_color("#00000000").unwrap();
        assert!(approx_eq(c, [0.0, 0.0, 0.0, 0.0]));
    }

    // -- shortcut parser ---------------------------------------------------

    #[test]
    fn parse_shortcut_ctrl_shift_t() {
        let c = parse_shortcut("ctrl+shift+t").unwrap();
        assert_eq!(c.modifiers, ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(c.key, KeyMatch::Char('t'));
    }

    #[test]
    fn parse_shortcut_super_alone() {
        let c = parse_shortcut("super+v").unwrap();
        assert_eq!(c.modifiers, ModifiersState::SUPER);
        assert_eq!(c.key, KeyMatch::Char('v'));
    }

    #[test]
    fn parse_shortcut_uppercase_tokens() {
        let c = parse_shortcut("Ctrl+Shift+T").unwrap();
        assert_eq!(c.modifiers, ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(c.key, KeyMatch::Char('t'));
    }

    #[test]
    fn parse_shortcut_tab_named() {
        let c = parse_shortcut("ctrl+tab").unwrap();
        assert_eq!(c.modifiers, ModifiersState::CONTROL);
        assert_eq!(c.key, KeyMatch::Tab);
    }

    #[test]
    fn parse_shortcut_function_key() {
        let c = parse_shortcut("f2").unwrap();
        assert_eq!(c.modifiers, ModifiersState::empty());
        assert_eq!(c.key, KeyMatch::Function(2));
    }

    #[test]
    fn parse_shortcut_unknown_token_errors() {
        let r = parse_shortcut("ctrl+blorp");
        assert!(r.is_err());
    }

    #[test]
    fn parse_shortcut_no_key_token_errors() {
        let r = parse_shortcut("ctrl+shift");
        assert!(r.is_err());
    }

    #[test]
    fn parse_shortcut_function_key_out_of_range() {
        let r = parse_shortcut("f15");
        assert!(r.is_err());
    }

    // -- Config::default + load -------------------------------------------

    #[test]
    fn default_values_have_8_shortcut_actions() {
        let cfg = Config::default_values();
        // 8 actions: NewTab CloseTab NextTab PrevTab RestartTab Copy Paste RenameTab
        assert_eq!(cfg.shortcuts.bindings.len(), 8);
    }

    #[test]
    fn load_missing_file_returns_defaults_no_errors() {
        let (cfg, errs) = Config::load(Path::new("/nonexistent/path/config.toml"));
        assert!(errs.is_empty());
        assert_eq!(cfg.cursor.blink_ms, 500);
    }

    #[test]
    fn load_partial_file_overrides_only_named_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"[cursor]
blink_ms = 250
"#
        )
        .unwrap();
        drop(f);

        let (cfg, errs) = Config::load(&path);
        assert!(errs.is_empty(), "errors: {errs:?}");
        assert_eq!(cfg.cursor.blink_ms, 250);
        // Other keys still defaults.
        assert!(approx_eq(cfg.colors.selection, [0.4, 0.6, 1.0, 0.4]));
    }

    #[test]
    fn load_bad_color_collects_error_keeps_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[colors]\nselection = \"not a color\"\n").unwrap();

        let (cfg, errs) = Config::load(&path);
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ConfigError::InvalidColor { key, .. } => assert_eq!(key, "selection"),
            other => panic!("expected InvalidColor, got {other:?}"),
        }
        // Default selection color preserved.
        assert!(approx_eq(cfg.colors.selection, [0.4, 0.6, 1.0, 0.4]));
    }

    #[test]
    fn load_bad_shortcut_collects_error_keeps_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[shortcuts]\nnew_tab = [\"ctrl+blorp\"]\n").unwrap();

        let (cfg, errs) = Config::load(&path);
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ConfigError::InvalidShortcut { action, .. } => assert_eq!(action, "new_tab"),
            other => panic!("expected InvalidShortcut, got {other:?}"),
        }
        // Every spec failed to parse → fall back to the default binding
        // (ctrl+shift+t and super+t). Otherwise a typo would silently disable
        // the action.
        assert_eq!(
            cfg.shortcuts.bindings.get(&Shortcut::NewTab).map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn explicit_empty_shortcut_list_disables_action() {
        // `new_tab = []` is an explicit "disable this action" — distinct from
        // a typo (which keeps the default per the previous test).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[shortcuts]\nnew_tab = []\n").unwrap();

        let (cfg, errs) = Config::load(&path);
        assert!(errs.is_empty(), "errors: {errs:?}");
        assert_eq!(
            cfg.shortcuts.bindings.get(&Shortcut::NewTab).map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn load_unknown_action_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[shortcuts]\nlaunch_rocket = [\"ctrl+r\"]\n").unwrap();

        let (_cfg, errs) = Config::load(&path);
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ConfigError::UnknownAction(name) => assert_eq!(name, "launch_rocket"),
            other => panic!("expected UnknownAction, got {other:?}"),
        }
    }

    #[test]
    fn load_syntax_error_reports_line_col() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[colors]\nselection = #not-quoted\n").unwrap();

        let (cfg, errs) = Config::load(&path);
        assert_eq!(errs.len(), 1);
        matches!(&errs[0], ConfigError::Syntax { .. });
        // Defaults preserved.
        assert_eq!(cfg.cursor.blink_ms, 500);
    }
}
