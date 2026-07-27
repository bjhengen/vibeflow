//! TOML configuration: schema types, parsing, hot-reload, and the
//! `AppUserEvent` enum delivered via `EventLoopProxy::send_event` from the
//! file-watcher thread to `WindowApp::user_event` on the main thread.

pub mod bounds;
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
    /// Stage 13: name of the active color preset (from `[colors] preset = "…"`).
    /// Stored here (not in `Colors`) so `Colors` can remain `Copy`.
    pub color_preset: Option<String>,
    pub cursor: CursorConfig,
    pub fonts: FontsConfig,
    pub clipboard: ClipboardConfig,
    pub tabs: TabsConfig,
    pub ai: Ai,
    pub scrollback: Scrollback,
    pub bell: Bell,
    pub ui: Ui,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ai {
    pub tools: Vec<String>,
    pub heuristic_silence_ms: u64,
    pub stale_state_timeout_s: u64,
    pub debounce_ms: u64,
    pub foreground_check_interval_ms: u64,
    pub explicit_stale_state_s: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scrollback {
    pub history_lines: u32,
    pub wheel_lines_per_detent: u32,
    pub scrollbar_fade_ms: u64,
    pub snap_on_esc: bool,
}

/// Terminal bell behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BellMode {
    Visual,
    Audible,
    Both,
    Silent,
}

impl std::str::FromStr for BellMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "visual" => Ok(BellMode::Visual),
            "audible" => Ok(BellMode::Audible),
            "both" => Ok(BellMode::Both),
            "silent" => Ok(BellMode::Silent),
            other => Err(format!("unknown bell mode: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bell {
    pub mode: BellMode,
    pub debounce_ms: u64,
}

/// Resolved `[ui]` configuration. v0.1.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ui {
    pub confirm_on_close: bool,
    /// When `true` (default), a `Waiting` tab's amber indicator pulses (a
    /// 1.4 s sine animation). Each pulse frame re-renders → forces a
    /// full-surface present; under a software X server (VNC/remote) that is
    /// re-encoded as full-screen damage and reads as flicker (#19). Set
    /// `false` for a steady (non-animating) amber indicator with no such
    /// repaint cost.
    pub indicator_pulse: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            confirm_on_close: true,
            indicator_pulse: true,
        }
    }
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
    /// `Key::Named(NamedKey::PageUp)`.
    PageUp,
    /// `Key::Named(NamedKey::PageDown)`.
    PageDown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub selection: [f32; 4],
    pub indicator_active: [f32; 4],
    pub indicator_working: [f32; 4],
    pub indicator_waiting: [f32; 4],
    pub indicator_inactive: [f32; 4],
    pub menu_bg: [f32; 4],
    pub menu_border: [f32; 4],
    pub menu_text: [f32; 4],
    pub menu_text_disabled: [f32; 4],
    pub menu_shortcut: [f32; 4],
    pub menu_focus_bg: [f32; 4],
    pub scrollbar_track: [f32; 4],
    pub scrollbar_thumb: [f32; 4],
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
    /// When false, OSC 52 clipboard-write requests from terminal output are
    /// ignored so untrusted output cannot overwrite the system clipboard.
    /// Default `true`. OSC 52 *read* is never implemented regardless.
    pub allow_osc52_write: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabsConfig {
    /// When false, OSC 0/2 from shells is silently ignored. Tab titles stay
    /// at the spawned shell name until the user renames manually. Default `true`.
    pub respect_osc_title: bool,
    /// Strip this prefix from any incoming OSC 0/2 title before displaying.
    /// Empty = no stripping. Default empty.
    pub title_strip_prefix: String,
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
                menu_bg: rgba(0x1a, 0x1a, 0x22, 0xFF),
                menu_border: rgba(0x2a, 0x2a, 0x35, 0xFF),
                menu_text: rgba(0xe8, 0xe8, 0xec, 0xFF),
                menu_text_disabled: rgba(0x5a, 0x5a, 0x65, 0xFF),
                menu_shortcut: rgba(0x99, 0x99, 0xa5, 0xFF),
                menu_focus_bg: rgba(0x2a, 0x35, 0x50, 0xFF),
                scrollbar_track: [1.0, 1.0, 1.0, 0.04],
                scrollbar_thumb: [1.0, 1.0, 1.0, 0.22],
            },
            color_preset: None,
            cursor: CursorConfig { blink_ms: 500 },
            fonts: FontsConfig {
                priority: vec![
                    "JetBrains Mono".to_string(),
                    "Noto Color Emoji".to_string(),
                    "DejaVu Sans Mono".to_string(),
                ],
            },
            clipboard: ClipboardConfig {
                primary: true,
                allow_osc52_write: true,
            },
            tabs: TabsConfig {
                respect_osc_title: true,
                title_strip_prefix: String::new(),
            },
            ai: Ai {
                tools: vec![
                    "claude".to_owned(),
                    "codex".to_owned(),
                    "opencode".to_owned(),
                    "aider".to_owned(),
                    "cursor-agent".to_owned(),
                ],
                heuristic_silence_ms: 4000,
                stale_state_timeout_s: 30,
                debounce_ms: 100,
                foreground_check_interval_ms: 250,
                explicit_stale_state_s: 300,
            },
            scrollback: Scrollback {
                history_lines: 10000,
                wheel_lines_per_detent: 3,
                scrollbar_fade_ms: 1500,
                snap_on_esc: true,
            },
            bell: Bell {
                mode: BellMode::Visual,
                debounce_ms: 100,
            },
            ui: Ui::default(),
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
            defaults.color_preset = c.preset.clone();
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
            if let Some(a) = cb.allow_osc52_write {
                defaults.clipboard.allow_osc52_write = a;
            }
        }
        if let Some(t) = file.tabs {
            if let Some(b) = t.respect_osc_title {
                defaults.tabs.respect_osc_title = b;
            }
            if let Some(p) = t.title_strip_prefix {
                defaults.tabs.title_strip_prefix = p;
            }
        }
        if let Some(a) = file.ai {
            apply_ai(a, &mut defaults.ai);
        }
        if let Some(s) = file.scrollback {
            apply_scrollback(s, &mut defaults.scrollback);
        }
        if let Some(b) = file.bell {
            apply_bell(b, &mut defaults.bell, &mut errors);
        }
        if let Some(u) = file.ui {
            apply_ui(u, &mut defaults.ui);
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
            "pageup" | "pgup" => {
                if key.is_some() {
                    return Err(format!("multiple key tokens in \"{s}\""));
                }
                key = Some(KeyMatch::PageUp);
            }
            "pagedown" | "pgdn" => {
                if key.is_some() {
                    return Err(format!("multiple key tokens in \"{s}\""));
                }
                key = Some(KeyMatch::PageDown);
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
        (MoveTabLeft, &["ctrl+shift+pageup"]),
        (MoveTabRight, &["ctrl+shift+pagedown"]),
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
    apply(
        Shortcut::MoveTabLeft,
        "move_tab_left",
        section.move_tab_left,
    );
    apply(
        Shortcut::MoveTabRight,
        "move_tab_right",
        section.move_tab_right,
    );
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
    apply("menu_bg", &mut out.menu_bg, section.menu_bg);
    apply("menu_border", &mut out.menu_border, section.menu_border);
    apply("menu_text", &mut out.menu_text, section.menu_text);
    apply(
        "menu_text_disabled",
        &mut out.menu_text_disabled,
        section.menu_text_disabled,
    );
    apply(
        "menu_shortcut",
        &mut out.menu_shortcut,
        section.menu_shortcut,
    );
    apply(
        "menu_focus_bg",
        &mut out.menu_focus_bg,
        section.menu_focus_bg,
    );
    apply(
        "scrollbar_track",
        &mut out.scrollbar_track,
        section.scrollbar_track,
    );
    apply(
        "scrollbar_thumb",
        &mut out.scrollbar_thumb,
        section.scrollbar_thumb,
    );
}

fn apply_ai(schema: schema::AiSection, resolved: &mut Ai) {
    use crate::config::bounds::{
        clamp_with_warn, AI_DEBOUNCE_MS_MAX, AI_DEBOUNCE_MS_MIN, AI_EXPLICIT_STALE_STATE_S_MAX,
        AI_EXPLICIT_STALE_STATE_S_MIN, AI_FOREGROUND_CHECK_INTERVAL_MS_MAX,
        AI_FOREGROUND_CHECK_INTERVAL_MS_MIN, AI_HEURISTIC_SILENCE_MS_MAX,
        AI_HEURISTIC_SILENCE_MS_MIN, AI_STALE_STATE_TIMEOUT_S_MAX, AI_STALE_STATE_TIMEOUT_S_MIN,
    };
    if let Some(tools) = schema.tools {
        resolved.tools = tools;
    }
    if let Some(v) = schema.heuristic_silence_ms {
        resolved.heuristic_silence_ms = clamp_with_warn(
            "ai.heuristic_silence_ms",
            v,
            AI_HEURISTIC_SILENCE_MS_MIN,
            AI_HEURISTIC_SILENCE_MS_MAX,
        );
    }
    if let Some(v) = schema.stale_state_timeout_s {
        resolved.stale_state_timeout_s = clamp_with_warn(
            "ai.stale_state_timeout_s",
            v,
            AI_STALE_STATE_TIMEOUT_S_MIN,
            AI_STALE_STATE_TIMEOUT_S_MAX,
        );
    }
    if let Some(v) = schema.debounce_ms {
        resolved.debounce_ms =
            clamp_with_warn("ai.debounce_ms", v, AI_DEBOUNCE_MS_MIN, AI_DEBOUNCE_MS_MAX);
    }
    if let Some(v) = schema.foreground_check_interval_ms {
        resolved.foreground_check_interval_ms = clamp_with_warn(
            "ai.foreground_check_interval_ms",
            v,
            AI_FOREGROUND_CHECK_INTERVAL_MS_MIN,
            AI_FOREGROUND_CHECK_INTERVAL_MS_MAX,
        );
    }
    if let Some(v) = schema.explicit_stale_state_s {
        // 0 disables the fuse per existing semantic; skip clamp at 0.
        resolved.explicit_stale_state_s = if v == 0 {
            0
        } else {
            clamp_with_warn(
                "ai.explicit_stale_state_s",
                v,
                AI_EXPLICIT_STALE_STATE_S_MIN,
                AI_EXPLICIT_STALE_STATE_S_MAX,
            )
        };
    }
}

fn apply_bell(schema: schema::BellSection, resolved: &mut Bell, errors: &mut Vec<ConfigError>) {
    use crate::config::bounds::{clamp_with_warn, BELL_DEBOUNCE_MS_MAX, BELL_DEBOUNCE_MS_MIN};
    if let Some(m) = schema.mode {
        match m.parse::<BellMode>() {
            Ok(mode) => resolved.mode = mode,
            Err(e) => errors.push(ConfigError::InvalidValue {
                key: "bell.mode".to_string(),
                expected: "visual | audible | both | silent".to_string(),
                got: e,
            }),
        }
    }
    if let Some(v) = schema.debounce_ms {
        resolved.debounce_ms = clamp_with_warn(
            "bell.debounce_ms",
            v,
            BELL_DEBOUNCE_MS_MIN,
            BELL_DEBOUNCE_MS_MAX,
        );
    }
}

fn apply_ui(schema: schema::UiSection, resolved: &mut Ui) {
    if let Some(v) = schema.confirm_on_close {
        resolved.confirm_on_close = v;
    }
    if let Some(v) = schema.indicator_pulse {
        resolved.indicator_pulse = v;
    }
}

fn apply_scrollback(schema: schema::ScrollbackSection, resolved: &mut Scrollback) {
    use crate::config::bounds::{
        clamp_with_warn, SCROLLBACK_HISTORY_LINES_MAX, SCROLLBACK_HISTORY_LINES_MIN,
        SCROLLBACK_SCROLLBAR_FADE_MS_MAX, SCROLLBACK_SCROLLBAR_FADE_MS_MIN,
        SCROLLBACK_WHEEL_LINES_PER_DETENT_MAX, SCROLLBACK_WHEEL_LINES_PER_DETENT_MIN,
    };
    if let Some(v) = schema.history_lines {
        resolved.history_lines = clamp_with_warn(
            "scrollback.history_lines",
            v,
            SCROLLBACK_HISTORY_LINES_MIN,
            SCROLLBACK_HISTORY_LINES_MAX,
        );
    }
    if let Some(v) = schema.wheel_lines_per_detent {
        resolved.wheel_lines_per_detent = clamp_with_warn(
            "scrollback.wheel_lines_per_detent",
            v,
            SCROLLBACK_WHEEL_LINES_PER_DETENT_MIN,
            SCROLLBACK_WHEEL_LINES_PER_DETENT_MAX,
        );
    }
    if let Some(v) = schema.scrollbar_fade_ms {
        resolved.scrollbar_fade_ms = clamp_with_warn(
            "scrollback.scrollbar_fade_ms",
            v,
            SCROLLBACK_SCROLLBAR_FADE_MS_MIN,
            SCROLLBACK_SCROLLBAR_FADE_MS_MAX,
        );
    }
    if let Some(v) = schema.snap_on_esc {
        resolved.snap_on_esc = v;
    }
}

/// Events delivered to the main thread via `EventLoopProxy::send_event`. The
/// only sender is the file-watcher thread (Task 6).
#[derive(Debug, Clone)]
pub enum AppUserEvent {
    /// New `Config` (with all defaults applied) plus any errors encountered
    /// during parse. `errors.is_empty()` means a clean reload.
    ///
    /// `Config` is boxed to keep the enum variant sizes comparable; the struct
    /// grows with each stage as more color/font fields are added.
    ConfigReloaded {
        config: Box<Config>,
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

    #[test]
    fn parse_shortcut_pageup_pagedown_tokens() {
        let c = parse_shortcut("ctrl+shift+pageup").unwrap();
        assert_eq!(c.key, KeyMatch::PageUp);
        assert!(c.modifiers.control_key() && c.modifiers.shift_key());
        assert_eq!(parse_shortcut("pgdn").unwrap().key, KeyMatch::PageDown);
        assert!(parse_shortcut("pageup+tab").is_err(), "two key tokens");
    }

    #[test]
    fn move_tab_shortcuts_load_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[shortcuts]\nmove_tab_left = [\"ctrl+alt+pageup\"]\n",
        )
        .unwrap();
        let (cfg, errors) = Config::load(&path);
        assert!(errors.is_empty(), "{errors:?}");
        let chords = cfg.shortcuts.bindings.get(&Shortcut::MoveTabLeft).unwrap();
        assert_eq!(chords.len(), 1);
        assert_eq!(chords[0].key, KeyMatch::PageUp);
    }

    // -- Config::default + load -------------------------------------------

    #[test]
    fn default_values_have_10_shortcut_actions() {
        let cfg = Config::default_values();
        // 10 actions: NewTab CloseTab NextTab PrevTab RestartTab Copy Paste
        // RenameTab MoveTabLeft MoveTabRight
        assert_eq!(cfg.shortcuts.bindings.len(), 10);
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
    fn default_tabs_respect_osc_title_is_true() {
        let cfg = Config::default_values();
        assert!(cfg.tabs.respect_osc_title);
    }

    #[test]
    fn load_tabs_respect_osc_title_false_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tabs]\nrespect_osc_title = false\n").unwrap();
        let (cfg, errs) = Config::load(&path);
        assert!(errs.is_empty(), "errors: {errs:?}");
        assert!(!cfg.tabs.respect_osc_title);
    }

    #[test]
    fn default_clipboard_allow_osc52_write_is_true() {
        let cfg = Config::default_values();
        assert!(cfg.clipboard.allow_osc52_write);
    }

    #[test]
    fn load_clipboard_allow_osc52_write_false_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[clipboard]\nallow_osc52_write = false\n").unwrap();
        let (cfg, errs) = Config::load(&path);
        assert!(errs.is_empty(), "errors: {errs:?}");
        assert!(!cfg.clipboard.allow_osc52_write);
    }

    #[test]
    fn load_tabs_title_strip_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tabs]\ntitle_strip_prefix = \"user@host: \"\n").unwrap();
        let (cfg, errs) = Config::load(&path);
        assert!(errs.is_empty(), "errors: {errs:?}");
        assert_eq!(cfg.tabs.title_strip_prefix, "user@host: ");
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

    #[test]
    fn menu_colors_default_to_dark_theme_values() {
        let cf = Config::default_values();
        assert_eq!(
            cf.colors.menu_bg,
            [
                0x1a as f32 / 255.0,
                0x1a as f32 / 255.0,
                0x22 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(
            cf.colors.menu_border,
            [
                0x2a as f32 / 255.0,
                0x2a as f32 / 255.0,
                0x35 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(
            cf.colors.menu_text,
            [
                0xe8 as f32 / 255.0,
                0xe8 as f32 / 255.0,
                0xec as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(
            cf.colors.menu_text_disabled,
            [
                0x5a as f32 / 255.0,
                0x5a as f32 / 255.0,
                0x65 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(
            cf.colors.menu_shortcut,
            [
                0x99 as f32 / 255.0,
                0x99 as f32 / 255.0,
                0xa5 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(
            cf.colors.menu_focus_bg,
            [
                0x2a as f32 / 255.0,
                0x35 as f32 / 255.0,
                0x50 as f32 / 255.0,
                1.0
            ]
        );
    }

    #[test]
    fn menu_colors_load_from_toml_overrides() {
        // Write a temp TOML file and load via Config::load (the public path users go through).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r##"
[colors]
menu_bg            = "#000000ff"
menu_focus_bg      = "#0000ffff"
"##,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(cf.colors.menu_bg, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(cf.colors.menu_focus_bg, [0.0, 0.0, 1.0, 1.0]);
        // Other fields keep their dark-theme defaults.
        assert_eq!(
            cf.colors.menu_border,
            [
                0x2a as f32 / 255.0,
                0x2a as f32 / 255.0,
                0x35 as f32 / 255.0,
                1.0
            ]
        );
    }

    #[test]
    fn ai_defaults_match_spec() {
        let cf = Config::default_values();
        assert_eq!(
            cf.ai.tools,
            vec![
                "claude".to_owned(),
                "codex".to_owned(),
                "opencode".to_owned(),
                "aider".to_owned(),
                "cursor-agent".to_owned(),
            ]
        );
        assert_eq!(cf.ai.heuristic_silence_ms, 4000);
        assert_eq!(cf.ai.stale_state_timeout_s, 30);
        assert_eq!(cf.ai.debounce_ms, 100);
        assert_eq!(cf.ai.foreground_check_interval_ms, 250);
    }

    #[test]
    fn ai_load_overrides_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[ai]
tools = ["mytool", "claude"]
heuristic_silence_ms = 2500
"#,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(cf.ai.tools, vec!["mytool".to_owned(), "claude".to_owned()]);
        assert_eq!(cf.ai.heuristic_silence_ms, 2500);
        // Other fields keep their defaults.
        assert_eq!(cf.ai.stale_state_timeout_s, 30);
        assert_eq!(cf.ai.debounce_ms, 100);
        assert_eq!(cf.ai.foreground_check_interval_ms, 250);
    }

    #[test]
    fn scrollback_defaults_match_spec() {
        let cf = Config::default_values();
        assert_eq!(cf.scrollback.history_lines, 10000);
        assert_eq!(cf.scrollback.wheel_lines_per_detent, 3);
        assert_eq!(cf.scrollback.scrollbar_fade_ms, 1500);
    }

    #[test]
    fn scrollback_load_overrides_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[scrollback]
history_lines = 500
wheel_lines_per_detent = 1
"#,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.scrollback.history_lines, 500);
        assert_eq!(cf.scrollback.wheel_lines_per_detent, 1);
        // Unspecified keys keep defaults.
        assert_eq!(cf.scrollback.scrollbar_fade_ms, 1500);
    }

    #[test]
    fn scrollback_history_lines_zero_clamps_to_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[scrollback]
history_lines = 0
"#,
        )
        .expect("write");
        let (cf, _) = Config::load(&path);
        assert_eq!(
            cf.scrollback.history_lines, 1,
            "0 should clamp to 1 per spec edge case"
        );
    }

    #[test]
    fn scrollbar_colors_default_to_subtle_white() {
        let cf = Config::default_values();
        assert_eq!(cf.colors.scrollbar_track, [1.0, 1.0, 1.0, 0.04]);
        assert_eq!(cf.colors.scrollbar_thumb, [1.0, 1.0, 1.0, 0.22]);
    }

    #[test]
    fn bell_defaults_match_spec() {
        let cf = Config::default_values();
        assert_eq!(cf.bell.mode, BellMode::Visual);
        assert_eq!(cf.bell.debounce_ms, 100);
    }

    #[test]
    fn bell_load_overrides_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[bell]
mode = "audible"
debounce_ms = 50
"#,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.bell.mode, BellMode::Audible);
        assert_eq!(cf.bell.debounce_ms, 50);
    }

    #[test]
    fn bell_mode_invalid_string_logs_error_keeps_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[bell]
mode = "blinking-lights"
"#,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(!errors.is_empty(), "expected error for invalid mode");
        assert_eq!(cf.bell.mode, BellMode::Visual); // default preserved
    }

    #[test]
    fn scrollback_snap_on_esc_defaults_true() {
        let cf = Config::default_values();
        assert!(cf.scrollback.snap_on_esc);
    }

    #[test]
    fn scrollback_snap_on_esc_load_overrides() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[scrollback]
snap_on_esc = false
"#,
        )
        .expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert!(!cf.scrollback.snap_on_esc);
    }

    #[test]
    fn color_preset_defaults_none() {
        let cf = Config::default_values();
        assert_eq!(cf.color_preset, None);
    }

    #[test]
    fn color_preset_load_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[colors]\npreset = \"gruvbox\"\n").expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.color_preset.as_deref(), Some("gruvbox"));
    }

    #[test]
    fn ai_explicit_stale_state_default_is_300() {
        let cf = Config::default_values();
        assert_eq!(cf.ai.explicit_stale_state_s, 300);
    }

    #[test]
    fn ai_explicit_stale_state_load_override_and_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ai]\nexplicit_stale_state_s = 0\n").expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.ai.explicit_stale_state_s, 0);
    }

    #[test]
    fn ai_explicit_stale_state_load_override_nonzero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ai]\nexplicit_stale_state_s = 120\n").expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.ai.explicit_stale_state_s, 120);
    }

    #[test]
    fn apply_ai_clamps_oversize_heuristic_silence_ms() {
        let schema = schema::AiSection {
            tools: None,
            heuristic_silence_ms: Some(100_000_000), // 100M ms; well past 60s max
            stale_state_timeout_s: None,
            debounce_ms: None,
            foreground_check_interval_ms: None,
            explicit_stale_state_s: None,
        };
        let mut resolved = Config::default_values().ai;
        apply_ai(schema, &mut resolved);
        assert_eq!(
            resolved.heuristic_silence_ms, 60_000,
            "must clamp to AI_HEURISTIC_SILENCE_MS_MAX"
        );
    }

    #[test]
    fn apply_ai_explicit_stale_zero_preserved() {
        let schema = schema::AiSection {
            tools: None,
            heuristic_silence_ms: None,
            stale_state_timeout_s: None,
            debounce_ms: None,
            foreground_check_interval_ms: None,
            explicit_stale_state_s: Some(0),
        };
        let mut resolved = Config::default_values().ai;
        apply_ai(schema, &mut resolved);
        assert_eq!(
            resolved.explicit_stale_state_s, 0,
            "0 disables fuse; must not be clamped"
        );
    }

    #[test]
    fn apply_scrollback_clamps_oversize_history_lines() {
        let schema = schema::ScrollbackSection {
            history_lines: Some(100_000_000),
            wheel_lines_per_detent: None,
            scrollbar_fade_ms: None,
            snap_on_esc: None,
        };
        let mut resolved = Config::default_values().scrollback;
        apply_scrollback(schema, &mut resolved);
        assert_eq!(
            resolved.history_lines, 1_000_000,
            "must clamp to SCROLLBACK_HISTORY_LINES_MAX"
        );
    }

    #[test]
    fn ui_default_is_confirm_on_close_true() {
        let cfg = Ui::default();
        assert!(cfg.confirm_on_close);
    }

    #[test]
    fn ui_default_is_indicator_pulse_true() {
        // #19: the pulse animation is on by default (native displays); users on
        // VNC/remote X can disable it via `[ui] indicator_pulse = false`.
        let cfg = Ui::default();
        assert!(cfg.indicator_pulse);
    }

    #[test]
    fn apply_ui_overrides_default_when_false() {
        let schema = crate::config::schema::UiSection {
            confirm_on_close: Some(false),
            indicator_pulse: None,
        };
        let mut resolved = Ui::default();
        apply_ui(schema, &mut resolved);
        assert!(!resolved.confirm_on_close);
    }

    #[test]
    fn apply_ui_indicator_pulse_override_false() {
        let schema = crate::config::schema::UiSection {
            confirm_on_close: None,
            indicator_pulse: Some(false),
        };
        let mut resolved = Ui::default();
        apply_ui(schema, &mut resolved);
        assert!(!resolved.indicator_pulse);
        assert!(
            resolved.confirm_on_close,
            "an unrelated None must leave its default unchanged"
        );
    }

    #[test]
    fn apply_ui_no_op_when_none() {
        let schema = crate::config::schema::UiSection {
            confirm_on_close: None,
            indicator_pulse: None,
        };
        let mut resolved = Ui::default();
        apply_ui(schema, &mut resolved);
        assert!(
            resolved.confirm_on_close,
            "None should leave default unchanged"
        );
        assert!(
            resolved.indicator_pulse,
            "None should leave default unchanged"
        );
    }
}
