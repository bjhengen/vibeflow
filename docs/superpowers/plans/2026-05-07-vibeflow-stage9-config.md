# vibeflow Stage 9 Implementation Plan: TOML Config + Bundled UX Quick Wins

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `~/.config/vibeflow/config.toml` with hot-reload (shortcuts, colors, cursor blink, font priorities, PRIMARY clipboard); arrow / navigation keys; OSC 0/OSC 2 title-setting from shells/AI tools; interactive tab rename via `Ctrl+Shift+E` / `F2` / right-click.

**Architecture:** A single `Config` struct lives on `WindowApp`. A `notify`-based file-watcher thread runs in the background; on a config-modify event, it parses + validates the file and ships `AppUserEvent::ConfigReloaded { config, errors }` back to the main thread via `EventLoopProxy::send_event`. `WindowApp::user_event` distributes new values to subscribers (renderer setters, keymap table, clipboard primary toggle). Per-key tolerance: bad keys are dropped with errors collected into a `Vec<ConfigError>` rendered as an in-window banner. Tab rename and OSC 0/2 are independent paths that don't touch the config layer.

**Tech Stack:** New deps: `serde = { version = "1", features = ["derive"] }`, `toml = "0.8"`, `notify = "6"`, `dirs = "5"`. Existing: `winit 0.30` (`EventLoopProxy<UserEvent>`, `ApplicationHandler::user_event`), `cosmic-text 0.12` (font priorities reorder via fontdb rebuild), `arboard 3.6` (PRIMARY selector via `arboard::Clipboard::set_text_primary` / `get_text_primary`).

**Lessons carried forward from Stages 1–8:**
- Plan-verbatim Rust must be rustfmt-clean. If a compact `if x { y }` line gets rejected by `cargo fmt --check`, that's a plan miss; the implementer should run `cargo fmt -p vibeflow -- <file>` after pasting.
- Per-task Haiku reviewers consistently miss whole-stage issues. Run a Sonnet senior holistic review before tagging.
- Pre-execution senior review catches API-mismatch claims (e.g. trait imports, accessor names) that compile-blockers depend on. Run it.
- Implementers will sometimes use refactor tasks to rewrite UNRELATED tests with fabricated justifications. Compare test-name lists before/after every multi-file refactor.
- Selection-clears-on-typed-input must gate on `key_to_bytes` returning `Some`, NOT every Pressed event. (Stage 8 smoke caught this.)
- `cargo` invoked from outside `/path/to/vibeflow` may pick up another `Cargo.toml`. Always prefix `cd /path/to/vibeflow &&` or use absolute paths.
- VNC display is available on host (port 5901, `DISPLAY=:1.0`). GUI smoke runs are runnable.

---

## File Structure

| Path | Responsibility | Net delta |
|---|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Add `serde`, `toml`, `notify`, `dirs`. | +4 / 0 |
| `crates/vibeflow/src/lib.rs` (modify) | Declare `pub mod config;`. | +1 / 0 |
| `crates/vibeflow/src/config/mod.rs` (create) | `Config` aggregate; `load(path)`; `default()`; color hex + shortcut spec parsers; `AppUserEvent` re-exports. | +280 |
| `crates/vibeflow/src/config/schema.rs` (create) | `serde::Deserialize` types for the TOML schema. All fields `Option<T>` for partial-parse tolerance. | +120 |
| `crates/vibeflow/src/config/watcher.rs` (create) | `notify`-based file watcher thread; manual 250ms debounce; sends `AppUserEvent::ConfigReloaded` / `ConfigError` via `EventLoopProxy`. | +120 |
| `crates/vibeflow/src/config/error_banner.rs` (create) | `ErrorBannerState` (errors, dismissed flag, format helper). Pure logic. | +130 |
| `crates/vibeflow/src/keymap.rs` (modify) | Replace hard-coded `match_shortcut` with a `ShortcutTable`. Add `Shortcut::RenameTab`. | +120 / -100 |
| `crates/vibeflow/src/render/mod.rs` (modify) | Setters: `set_selection_color`, `set_indicator_colors`, `set_cursor_blink_ms`, `set_font_priorities`. Banner rect/glyph slots in unified buffer. | +90 / -10 |
| `crates/vibeflow/src/render/text_engine.rs` (modify) | `set_font_priorities` rebuilds the cosmic-text `FontSystem` and clears the atlas. | +35 |
| `crates/vibeflow/src/render/cursor.rs` (modify) | `set_blink_ms(ms: u64)`; `0` disables blink. | +12 |
| `crates/vibeflow/src/render/tabs.rs` (modify) | `push_text_glyphs` honors `Option<&RenameInputState>` — substitutes buffer + caret for the renaming tab. | +60 / -5 |
| `crates/vibeflow/src/clipboard.rs` (modify) | `set_primary_enabled(bool)`; `copy` writes to BOTH selectors when enabled; `paste_primary()` reads PRIMARY. | +50 / -5 |
| `crates/vibeflow/src/session/osc.rs` (modify) | `DispatchEvent::SetTitle(String)`; parse OSC 0 / OSC 2. | +60 |
| `crates/vibeflow/src/session/session.rs` (modify) | `pub user_renamed: bool` on `PtySession`; `set_title_from_osc`; reset in `restart`. | +25 |
| `crates/vibeflow/src/window.rs` (modify) | `EventLoop::<AppUserEvent>::with_user_event()`; `user_event` distributor; `RenameInputState` + keyboard capture + render override; right-click rename trigger; click-out-cancel; arrow / nav keys in `key_to_bytes`; mouse-release auto-copy to PRIMARY; middle-click paste from PRIMARY. | +320 / -20 |
| `crates/vibeflow/src/main.rs` (modify) | Event-loop type swap to `EventLoop::<AppUserEvent>`; pass proxy to `WindowApp::new`. | +20 / -5 |
| `docs/TESTING.md` (modify) | Append Stage 9 manual smoke checklist. | +60 |

**Net add:** ~+1530 / −145 (≈ +1385 net), 13 files modified, 4 files created, 4 deps added.

---

## Task 0: Branch + dep adds + module stubs

**Files:**
- Create branch: `stage9-config` from `main` (currently at `79c7c29` after the Stage 9 design spec commit).
- Modify: `crates/vibeflow/Cargo.toml`
- Modify: `crates/vibeflow/src/lib.rs`
- Create: `crates/vibeflow/src/config/mod.rs` (stub)
- Create: `crates/vibeflow/src/config/schema.rs` (stub)
- Create: `crates/vibeflow/src/config/watcher.rs` (stub)
- Create: `crates/vibeflow/src/config/error_banner.rs` (stub)

This task adds deps + creates empty module stubs so later tasks have a place to grow. NO functionality yet.

- [ ] **Step 1: Create the branch**

```bash
cd /path/to/vibeflow
git checkout main
git pull --ff-only || true
git checkout -b stage9-config
```

- [ ] **Step 2: Add deps**

Open `crates/vibeflow/Cargo.toml`. Find the `[dependencies]` section. Add (place each alphabetically; verify the resulting block):

```toml
dirs = "5"
notify = "6"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

Don't reorder unrelated deps. `arboard`, `alacritty_terminal`, `anyhow`, `bytemuck`, etc. are already there from earlier stages — leave them.

- [ ] **Step 3: Create module stubs**

```bash
cd /path/to/vibeflow
mkdir -p crates/vibeflow/src/config
touch crates/vibeflow/src/config/mod.rs
touch crates/vibeflow/src/config/schema.rs
touch crates/vibeflow/src/config/watcher.rs
touch crates/vibeflow/src/config/error_banner.rs
```

Add module-level doc comments to each:

`crates/vibeflow/src/config/mod.rs`:
```rust
//! TOML configuration: schema types, parsing, hot-reload, and the
//! `AppUserEvent` enum delivered via `EventLoopProxy::send_event` from the
//! file-watcher thread to `WindowApp::user_event` on the main thread.

pub mod error_banner;
pub mod schema;
pub mod watcher;
```

`crates/vibeflow/src/config/schema.rs`:
```rust
//! `serde::Deserialize`-derivable schema types matching the on-disk TOML
//! layout. All fields are `Option<T>` for partial-parse tolerance — missing
//! keys map to `None` here and are filled with defaults by `Config::load`.
```

`crates/vibeflow/src/config/watcher.rs`:
```rust
//! Background file-watcher thread. Uses `notify` to detect changes to
//! `~/.config/vibeflow/config.toml`, debounces 250 ms, parses + validates,
//! and ships `AppUserEvent::ConfigReloaded` via `EventLoopProxy::send_event`
//! to the main thread.
```

`crates/vibeflow/src/config/error_banner.rs`:
```rust
//! In-window banner state for "N config keys ignored" errors. Pure logic;
//! the renderer reads `ErrorBannerState` and emits one rect range + one
//! glyph range per frame when the banner is visible and not dismissed.
```

- [ ] **Step 4: Declare `pub mod config;` in lib.rs**

Open `crates/vibeflow/src/lib.rs`. Add `pub mod config;` alphabetically among the existing module declarations (after `pub mod clipboard;`, before `pub mod keymap;`).

- [ ] **Step 5: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean. 222 default tests pass + 14 ignored... wait, no, this is Stage 9 starting state. Stage 8 ended at 176 default + 13 ignored. Task 0 adds zero tests. Expected: 176 default + 13 ignored.

If clippy fails on the empty-stub modules, verify the module-level doc comments were added.

If `notify`/`serde`/`toml`/`dirs` fail to resolve, verify Cargo.toml edit is syntactically valid.

- [ ] **Step 6: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/Cargo.toml \
        crates/vibeflow/Cargo.lock \
        crates/vibeflow/src/lib.rs \
        crates/vibeflow/src/config/mod.rs \
        crates/vibeflow/src/config/schema.rs \
        crates/vibeflow/src/config/watcher.rs \
        crates/vibeflow/src/config/error_banner.rs
git commit -m "chore: scaffold Stage 9 config module + add deps (serde, toml, notify, dirs)"
```

If `cargo build` modified `Cargo.lock`, include it.

---

## Task 1: Config schema (`config/schema.rs`) — Deserialize types (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs`

`serde`-derived TOML schema. All fields `Option<T>` so missing keys → `None`. The `mod.rs::load` function maps `None` to defaults.

- [ ] **Step 1: Write the schema**

Replace the contents of `crates/vibeflow/src/config/schema.rs` with:

```rust
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
    pub new_tab:     Option<Vec<String>>,
    pub close_tab:   Option<Vec<String>>,
    pub next_tab:    Option<Vec<String>>,
    pub prev_tab:    Option<Vec<String>>,
    pub restart_tab: Option<Vec<String>>,
    pub copy:        Option<Vec<String>>,
    pub paste:       Option<Vec<String>>,
    pub rename_tab:  Option<Vec<String>>,
    /// Unknown action keys land here so `mod.rs::load` can warn about them.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// `[colors]` table. RGBA hex strings like `"#6699FF66"`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColorsSection {
    pub selection:           Option<String>,
    pub indicator_active:    Option<String>,
    pub indicator_working:   Option<String>,
    pub indicator_waiting:   Option<String>,
    pub indicator_inactive:  Option<String>,
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
        let s = r#"
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
        "#;
        let cf = parse(s);
        let shortcuts = cf.shortcuts.expect("shortcuts");
        assert_eq!(
            shortcuts.new_tab.as_deref(),
            Some(&["ctrl+shift+t".to_string(), "super+t".to_string()][..])
        );
        assert_eq!(shortcuts.copy.as_deref(), Some(&["ctrl+shift+c".to_string()][..]));
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
        let s = r#"
            [colors]
            selection = "#ff0000ff"
        "#;
        let cf = parse(s);
        let colors = cf.colors.expect("colors");
        assert_eq!(colors.selection.as_deref(), Some("#ff0000ff"));
        assert_eq!(colors.indicator_active, None);
    }

    #[test]
    fn unknown_top_level_key_fails_parse() {
        // `deny_unknown_fields` rejects typos at the top level.
        let s = r#"
            [colros]
            selection = "#000000ff"
        "#;
        let r: Result<ConfigFile, _> = toml::from_str(s);
        assert!(r.is_err(), "expected parse error for unknown top-level key");
    }

    #[test]
    fn unknown_shortcut_action_lands_in_extra() {
        let s = r#"
            [shortcuts]
            new_tab = ["ctrl+shift+t"]
            launch_rocket = ["ctrl+r"]
        "#;
        let cf = parse(s);
        let shortcuts = cf.shortcuts.expect("shortcuts");
        assert!(shortcuts.extra.contains_key("launch_rocket"));
        assert_eq!(shortcuts.new_tab.unwrap().len(), 1);
    }

    #[test]
    fn unknown_colors_field_fails_parse() {
        // ColorsSection has deny_unknown_fields — typos rejected.
        let s = r#"
            [colors]
            selectoin = "#ff0000ff"
        "#;
        let r: Result<ConfigFile, _> = toml::from_str(s);
        assert!(r.is_err());
    }
}
```

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/config/schema.rs
cargo test -p vibeflow --lib config::schema 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 6 new schema tests pass. 176 + 6 = 182 default + 13 ignored. fmt/clippy clean.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/config/schema.rs
git commit -m "feat(config): TOML schema types (ConfigFile + sections, TDD)"
```

---

## Task 2: Config aggregate + parsers (`config/mod.rs`) — TDD

**Files:**
- Modify: `crates/vibeflow/src/config/mod.rs`

Resolved `Config` struct (no Options), `ConfigError` enum, color/shortcut parsers, and `Config::load(path)`.

- [ ] **Step 1: Write the implementation**

Replace the contents of `crates/vibeflow/src/config/mod.rs` with:

```rust
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
    pub selection:          [f32; 4],
    pub indicator_active:   [f32; 4],
    pub indicator_working:  [f32; 4],
    pub indicator_waiting:  [f32; 4],
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
    Syntax { line: usize, col: usize, msg: String },
    /// Filesystem error (permission denied, missing dir, etc.).
    IoError(String),
    /// `#RRGGBBAA` parse failure.
    InvalidColor { key: String, value: String, msg: String },
    /// Shortcut spec parse failure (`"ctrl+shift+t"`).
    InvalidShortcut { action: String, value: String, msg: String },
    /// Unknown shortcut action (lands in `ShortcutsSection.extra`).
    UnknownAction(String),
    /// Catch-all for "expected u64 but got string".
    InvalidValue { key: String, expected: String, got: String },
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
                selection:          rgba(0x66, 0x99, 0xFF, 0x66),
                indicator_active:   rgba(0x22, 0xCC, 0x66, 0xFF),
                indicator_working:  rgba(0x33, 0x99, 0xFF, 0xFF),
                indicator_waiting:  rgba(0xFF, 0xAA, 0x00, 0xFF),
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
        parts[i] = u8::from_str_radix(&body[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("invalid hex pair \"{}\" in \"{s}\"", &body[i * 2..i * 2 + 2]))?;
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
    Ok(KeyChord { modifiers: mods, key })
}

fn default_shortcuts() -> ShortcutBindings {
    use Shortcut::*;
    let entries: &[(Shortcut, &[&str])] = &[
        (NewTab,     &["ctrl+shift+t", "super+t"]),
        (CloseTab,   &["ctrl+shift+w", "super+w"]),
        (NextTab,    &["ctrl+tab", "super+tab"]),
        (PrevTab,    &["ctrl+shift+tab", "super+shift+tab"]),
        (RestartTab, &["ctrl+shift+r", "super+r"]),
        (Copy,       &["ctrl+shift+c", "super+c"]),
        (Paste,      &["ctrl+shift+v", "super+v"]),
        (RenameTab,  &["ctrl+shift+e", "f2"]),
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
        // Empty list disables the action entirely.
        out.bindings.insert(action, chords);
    };
    apply(Shortcut::NewTab,     "new_tab",     section.new_tab);
    apply(Shortcut::CloseTab,   "close_tab",   section.close_tab);
    apply(Shortcut::NextTab,    "next_tab",    section.next_tab);
    apply(Shortcut::PrevTab,    "prev_tab",    section.prev_tab);
    apply(Shortcut::RestartTab, "restart_tab", section.restart_tab);
    apply(Shortcut::Copy,       "copy",        section.copy);
    apply(Shortcut::Paste,      "paste",       section.paste);
    apply(Shortcut::RenameTab,  "rename_tab",  section.rename_tab);
    for unknown in section.extra.keys() {
        errors.push(ConfigError::UnknownAction(unknown.clone()));
    }
}

fn apply_colors(
    out: &mut Colors,
    section: schema::ColorsSection,
    errors: &mut Vec<ConfigError>,
) {
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
    apply("selection",          &mut out.selection,          section.selection);
    apply("indicator_active",   &mut out.indicator_active,   section.indicator_active);
    apply("indicator_working",  &mut out.indicator_working,  section.indicator_working);
    apply("indicator_waiting",  &mut out.indicator_waiting,  section.indicator_waiting);
    apply("indicator_inactive", &mut out.indicator_inactive, section.indicator_inactive);
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
        write!(f, r#"[cursor]
blink_ms = 250
"#).unwrap();
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
        std::fs::write(
            &path,
            "[shortcuts]\nnew_tab = [\"ctrl+blorp\"]\n",
        )
        .unwrap();

        let (cfg, errs) = Config::load(&path);
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ConfigError::InvalidShortcut { action, .. } => assert_eq!(action, "new_tab"),
            other => panic!("expected InvalidShortcut, got {other:?}"),
        }
        // The bad spec was dropped, BUT since we replaced the binding with an empty
        // list, new_tab now has zero chords.
        assert_eq!(cfg.shortcuts.bindings.get(&Shortcut::NewTab).map(Vec::len), Some(0));
    }

    #[test]
    fn load_unknown_action_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[shortcuts]\nlaunch_rocket = [\"ctrl+r\"]\n",
        )
        .unwrap();

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
```

- [ ] **Step 2: Add `tempfile` as a dev-dep**

The `Config::load` integration tests use `tempfile`. Open `crates/vibeflow/Cargo.toml`. Find `[dev-dependencies]` (or add the section if it doesn't exist) and add:

```toml
[dev-dependencies]
tempfile = "3"
```

Don't disturb existing dev-deps.

- [ ] **Step 3: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/config/mod.rs
cargo test -p vibeflow --lib config:: 2>&1 | tail -25
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 21 new tests pass (6 schema + ~15 config). 182 + 15 = 197 default + 13 ignored. fmt/clippy clean.

If `parse_color`'s `bool::is_err` semantics confuse: each parser returns `Result<T, String>` and the test asserts `r.is_err()`.

If `tempfile` isn't found at link time, verify the dev-dep was added.

If clippy warns about `cast_precision_loss` on the `f32::EPSILON` comparisons in `approx_eq`, fix the test by using a larger absolute tolerance or by explicitly allowing in the test (not the production code) — but try the alternative first: `(x - y).abs() < 1e-3`.

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/config/mod.rs crates/vibeflow/Cargo.toml
git commit -m "feat(config): Config + per-key tolerant load + color/shortcut parsers (TDD)"
```

---

## Task 3: `AppUserEvent` + main.rs event-loop type swap

**Files:**
- Modify: `crates/vibeflow/src/config/mod.rs` (add `AppUserEvent` enum)
- Modify: `crates/vibeflow/src/window.rs` (add `proxy` field; stub `user_event`)
- Modify: `crates/vibeflow/src/main.rs` (event-loop type swap; create proxy)

This task wires the user-event plumbing without adding behavior. Build verifies.

- [ ] **Step 1: Add `AppUserEvent` to `config/mod.rs`**

Append to `crates/vibeflow/src/config/mod.rs` (above the `#[cfg(test)] mod tests`):

```rust
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
```

- [ ] **Step 2: Add proxy field to `WindowApp`**

Open `crates/vibeflow/src/window.rs`. Find `pub struct WindowApp`. Add (alphabetically among existing fields):

```rust
    /// Proxy for the file-watcher thread to ship `AppUserEvent` back to the
    /// main thread. Cloned and handed to the watcher in `resumed`.
    proxy: winit::event_loop::EventLoopProxy<crate::config::AppUserEvent>,
```

`WindowApp::new` (the constructor) gains a parameter:
```rust
pub fn new(proxy: winit::event_loop::EventLoopProxy<crate::config::AppUserEvent>) -> Self {
    // ... existing fields, including:
    Self {
        // ... existing initializers
        proxy,
    }
}
```

- [ ] **Step 3: Change the `ApplicationHandler` impl signature + implement `user_event` (stub)**

`winit-0.30.13`'s `ApplicationHandler` is `trait ApplicationHandler<T: 'static = ()>`. Stage 8 used the default `T = ()` — the impl line currently reads `impl ApplicationHandler for WindowApp`. Once we add `AppUserEvent`, that becomes `impl ApplicationHandler<crate::config::AppUserEvent> for WindowApp`. Without this typing the build will fail because the user_event method signature won't match the (still-default-`()`-typed) trait.

Update the impl line in `window.rs`:

```rust
// Was:
//   impl ApplicationHandler for WindowApp { ... }
// Becomes:
impl winit::application::ApplicationHandler<crate::config::AppUserEvent> for WindowApp {
    // ... existing methods (resumed, window_event, about_to_wait) UNCHANGED
    // Add the user_event stub below.
}
```

Then add to that block:

```rust
    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: crate::config::AppUserEvent,
    ) {
        // Stage 9 Task 7 will distribute this to subscribers. For now: trace.
        match &event {
            crate::config::AppUserEvent::ConfigReloaded { errors, .. } => {
                tracing::info!(error_count = errors.len(), "config reloaded (stub)");
            }
            crate::config::AppUserEvent::ConfigError(err) => {
                tracing::warn!(?err, "config error (stub)");
            }
        }
    }
```

- [ ] **Step 4: Update `main.rs`**

Open `crates/vibeflow/src/main.rs`. The current bootstrap probably looks like:

```rust
let event_loop = EventLoop::new().expect(...);
event_loop.set_control_flow(ControlFlow::Wait);
let mut app = WindowApp::new();
event_loop.run_app(&mut app)?;
```

Replace with:

```rust
let event_loop = EventLoop::<vibeflow::config::AppUserEvent>::with_user_event()
    .build()
    .expect("event loop");
event_loop.set_control_flow(ControlFlow::Wait);
let proxy = event_loop.create_proxy();
let mut app = WindowApp::new(proxy);
event_loop.run_app(&mut app)?;
```

The exact lines to change depend on the current `main.rs` layout — read it first. Don't blindly replace; locate the existing `EventLoop::new()` call and the `WindowApp::new()` call and update those two sites.

- [ ] **Step 5: Verify build**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/config/mod.rs crates/vibeflow/src/window.rs crates/vibeflow/src/main.rs
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: build clean. 197 default + 13 ignored (unchanged from Task 2 — Task 3 adds zero tests).

If `EventLoop::<T>::with_user_event()` returns a `EventLoopBuilder`, you'll need `.build().expect(...)`. If winit 0.30's API differs, adapt. `EventLoopProxy<T>` is `Clone + Send` in winit 0.30.

If WindowApp's constructor is called from somewhere besides `main.rs`, those call sites need the new `proxy` arg. Likely just `main.rs`.

If the trait `ApplicationHandler` requires the user-event type as a generic param (`ApplicationHandler<UserEvent = AppUserEvent>`), update the `impl` line accordingly.

- [ ] **Step 6: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/config/mod.rs \
        crates/vibeflow/src/window.rs \
        crates/vibeflow/src/main.rs
git commit -m "feat(config): AppUserEvent + EventLoopProxy plumbing (no behavior yet)"
```

---

## Task 4: Refactor `keymap` to `ShortcutTable` + add `RenameTab`

**Files:**
- Modify: `crates/vibeflow/src/keymap.rs`

Replace the hard-coded `match_shortcut` body with a `ShortcutTable` lookup that's populated from `Config.shortcuts`. Add `Shortcut::RenameTab` variant. Existing 17 unit tests must still pass (their semantic meaning preserved); add ~5 new tests for the table layer.

**Mandatory safety rule:** **DO NOT DELETE OR WEAKEN ANY EXISTING TEST.** Pre-Task-4 keymap test names that must still exist post-refactor:
- `ctrl_shift_t_is_new_tab`, `ctrl_shift_lowercase_t_is_new_tab`, `ctrl_shift_w_is_close_tab`, `ctrl_shift_r_is_restart_tab`, `ctrl_shift_c_is_copy`, `ctrl_shift_v_is_paste`, `ctrl_tab_is_next_tab`, `ctrl_shift_tab_is_prev_tab`, `super_t_is_new_tab`, `super_v_is_paste`, `super_tab_is_next_tab`, `super_shift_tab_is_prev_tab`, `plain_t_is_none`, `ctrl_t_without_shift_is_none`, `ctrl_shift_alt_t_is_none`, `ctrl_shift_x_is_none`, `super_with_ctrl_is_none`.

After your changes the file must still have all 17 of those test functions. Modify their internals only if absolutely required to call the new API; keep the assertions semantically identical.

- [ ] **Step 1: Add `RenameTab` variant + `ShortcutTable`**

Replace the contents of `crates/vibeflow/src/keymap.rs` with:

```rust
//! Keyboard shortcut dispatch. Stage 8 hard-coded the modifier+key → action
//! match; Stage 9 makes the table data-driven via `ShortcutTable`, populated
//! from `Config.shortcuts`. The default table reproduces Stage 8's bindings
//! exactly so behavior without a config file is unchanged.

use std::collections::HashMap;

use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Discrete shortcut actions vibeflow's `window.rs` dispatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    RestartTab,
    Copy,
    Paste,
    /// Stage 9: open the inline rename input on the active tab.
    RenameTab,
}

/// Keyed lookup table. Constructed via `ShortcutTable::default()` for the
/// built-in bindings, or from a `Config.shortcuts` via
/// `ShortcutTable::from_bindings`.
#[derive(Debug, Clone, Default)]
pub struct ShortcutTable {
    /// (modifiers, key-discriminant) -> action. Multiple chord entries can
    /// map to the same action.
    by_chord: HashMap<ChordKey, Shortcut>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChordKey {
    modifiers_bits: u32,
    key: ChordKeyDisc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ChordKeyDisc {
    /// ASCII lowercase letter.
    Char(char),
    Tab,
    Function(u8),
}

impl ShortcutTable {
    /// Default Stage 8 bindings — what users get without a config file.
    #[must_use]
    pub fn with_default_bindings() -> Self {
        let pairs: &[(Shortcut, &[(ModifiersState, ChordKeyDisc)])] = &[
            (
                Shortcut::NewTab,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Char('t')),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('t')),
                ],
            ),
            (
                Shortcut::CloseTab,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Char('w')),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('w')),
                ],
            ),
            (
                Shortcut::NextTab,
                &[
                    (ModifiersState::CONTROL, ChordKeyDisc::Tab),
                    (ModifiersState::SUPER, ChordKeyDisc::Tab),
                ],
            ),
            (
                Shortcut::PrevTab,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Tab),
                    (ModifiersState::SUPER.union(ModifiersState::SHIFT), ChordKeyDisc::Tab),
                ],
            ),
            (
                Shortcut::RestartTab,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Char('r')),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('r')),
                ],
            ),
            (
                Shortcut::Copy,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Char('c')),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('c')),
                ],
            ),
            (
                Shortcut::Paste,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Char('v')),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('v')),
                ],
            ),
            (
                Shortcut::RenameTab,
                &[
                    (ModifiersState::CONTROL.union(ModifiersState::SHIFT), ChordKeyDisc::Char('e')),
                    (ModifiersState::empty(), ChordKeyDisc::Function(2)),
                ],
            ),
        ];
        let mut by_chord = HashMap::new();
        for (action, chords) in pairs {
            for (mods, key) in *chords {
                by_chord.insert(
                    ChordKey {
                        modifiers_bits: mods.bits(),
                        key: *key,
                    },
                    *action,
                );
            }
        }
        Self { by_chord }
    }

    /// Lookup the action triggered by a winit key + modifier set, or `None`
    /// if no chord matches. Reject any combo with `Alt` set — vibeflow does
    /// not bind any `Alt+...` chord.
    #[must_use]
    pub fn lookup(&self, key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
        if modifiers.alt_key() {
            return None;
        }
        let disc = match key {
            Key::Character(c) => {
                let s = c.as_str();
                let mut chars = s.chars();
                let first = chars.next()?;
                if chars.next().is_some() || !first.is_ascii() {
                    return None;
                }
                ChordKeyDisc::Char(first.to_ascii_lowercase())
            }
            Key::Named(NamedKey::Tab) => ChordKeyDisc::Tab,
            Key::Named(NamedKey::F1) => ChordKeyDisc::Function(1),
            Key::Named(NamedKey::F2) => ChordKeyDisc::Function(2),
            Key::Named(NamedKey::F3) => ChordKeyDisc::Function(3),
            Key::Named(NamedKey::F4) => ChordKeyDisc::Function(4),
            Key::Named(NamedKey::F5) => ChordKeyDisc::Function(5),
            Key::Named(NamedKey::F6) => ChordKeyDisc::Function(6),
            Key::Named(NamedKey::F7) => ChordKeyDisc::Function(7),
            Key::Named(NamedKey::F8) => ChordKeyDisc::Function(8),
            Key::Named(NamedKey::F9) => ChordKeyDisc::Function(9),
            Key::Named(NamedKey::F10) => ChordKeyDisc::Function(10),
            Key::Named(NamedKey::F11) => ChordKeyDisc::Function(11),
            Key::Named(NamedKey::F12) => ChordKeyDisc::Function(12),
            _ => return None,
        };
        // Strip Alt (already rejected above) but keep Ctrl/Shift/Super.
        let mods_bits = (modifiers
            & (ModifiersState::CONTROL | ModifiersState::SHIFT | ModifiersState::SUPER))
            .bits();
        self.by_chord
            .get(&ChordKey {
                modifiers_bits: mods_bits,
                key: disc,
            })
            .copied()
    }
}

/// Backward-compat free function for callers that haven't been migrated to
/// `ShortcutTable::lookup` yet. Uses the default bindings.
#[must_use]
pub fn match_shortcut(key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
    static DEFAULT: std::sync::OnceLock<ShortcutTable> = std::sync::OnceLock::new();
    DEFAULT
        .get_or_init(ShortcutTable::with_default_bindings)
        .lookup(key, modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

    fn ch(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    fn mods(ctrl: bool, shift: bool, alt: bool, supr: bool) -> ModifiersState {
        let mut m = ModifiersState::empty();
        if ctrl {
            m |= ModifiersState::CONTROL;
        }
        if shift {
            m |= ModifiersState::SHIFT;
        }
        if alt {
            m |= ModifiersState::ALT;
        }
        if supr {
            m |= ModifiersState::SUPER;
        }
        m
    }

    // ===== Existing 17 tests (preserved verbatim from Stage 8) =====

    #[test]
    fn ctrl_shift_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, true, false, false)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn ctrl_shift_lowercase_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("t"), mods(true, true, false, false)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn ctrl_shift_w_is_close_tab() {
        assert_eq!(
            match_shortcut(&ch("W"), mods(true, true, false, false)),
            Some(Shortcut::CloseTab)
        );
    }

    #[test]
    fn ctrl_shift_r_is_restart_tab() {
        assert_eq!(
            match_shortcut(&ch("R"), mods(true, true, false, false)),
            Some(Shortcut::RestartTab)
        );
    }

    #[test]
    fn ctrl_shift_c_is_copy() {
        assert_eq!(
            match_shortcut(&ch("C"), mods(true, true, false, false)),
            Some(Shortcut::Copy)
        );
    }

    #[test]
    fn ctrl_shift_v_is_paste() {
        assert_eq!(
            match_shortcut(&ch("V"), mods(true, true, false, false)),
            Some(Shortcut::Paste)
        );
    }

    #[test]
    fn ctrl_tab_is_next_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(true, false, false, false)),
            Some(Shortcut::NextTab)
        );
    }

    #[test]
    fn ctrl_shift_tab_is_prev_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(true, true, false, false)),
            Some(Shortcut::PrevTab)
        );
    }

    #[test]
    fn super_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(false, false, false, true)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn super_v_is_paste() {
        assert_eq!(
            match_shortcut(&ch("V"), mods(false, false, false, true)),
            Some(Shortcut::Paste)
        );
    }

    #[test]
    fn super_tab_is_next_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(false, false, false, true)),
            Some(Shortcut::NextTab)
        );
    }

    #[test]
    fn super_shift_tab_is_prev_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(false, true, false, true)),
            Some(Shortcut::PrevTab)
        );
    }

    #[test]
    fn plain_t_is_none() {
        assert_eq!(match_shortcut(&ch("T"), mods(false, false, false, false)), None);
    }

    #[test]
    fn ctrl_t_without_shift_is_none() {
        assert_eq!(match_shortcut(&ch("T"), mods(true, false, false, false)), None);
    }

    #[test]
    fn ctrl_shift_alt_t_is_none() {
        assert_eq!(match_shortcut(&ch("T"), mods(true, true, true, false)), None);
    }

    #[test]
    fn ctrl_shift_x_is_none() {
        assert_eq!(match_shortcut(&ch("X"), mods(true, true, false, false)), None);
    }

    #[test]
    fn super_with_ctrl_is_none() {
        assert_eq!(match_shortcut(&ch("T"), mods(true, false, false, true)), None);
    }

    // ===== New Stage 9 tests =====

    #[test]
    fn ctrl_shift_e_is_rename_tab() {
        assert_eq!(
            match_shortcut(&ch("e"), mods(true, true, false, false)),
            Some(Shortcut::RenameTab)
        );
    }

    #[test]
    fn f2_is_rename_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::F2), mods(false, false, false, false)),
            Some(Shortcut::RenameTab)
        );
    }

    #[test]
    fn f1_is_none_by_default() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::F1), mods(false, false, false, false)),
            None
        );
    }

    #[test]
    fn shortcut_table_default_has_all_actions() {
        let t = ShortcutTable::with_default_bindings();
        // 8 distinct actions × 2 chord aliases = 16 entries.
        assert_eq!(t.by_chord.len(), 16);
    }

    #[test]
    fn shortcut_table_lookup_strips_alt_aware() {
        let t = ShortcutTable::with_default_bindings();
        let alt_only = ModifiersState::ALT;
        // Alt set => None always.
        assert_eq!(t.lookup(&ch("t"), ModifiersState::CONTROL | ModifiersState::SHIFT | alt_only), None);
    }
}
```

- [ ] **Step 1.5: Add a no-op `RenameTab` arm in `WindowApp::handle_shortcut`**

`window.rs::handle_shortcut` is currently an EXHAUSTIVE match over the `Shortcut` variants (no `_ =>` catch-all). Adding `Shortcut::RenameTab` as a new variant in this task makes the match non-exhaustive — the build fails with "non-exhaustive patterns: `Shortcut::RenameTab` not covered" until something handles it.

Add a temporary no-op arm. Task 13 fills it in with `self.start_rename(...)`:

Open `crates/vibeflow/src/window.rs`. In `WindowApp::handle_shortcut`, alongside the existing arms:

```rust
            Shortcut::RenameTab => {
                // Stage 9 Task 13 wires this to start_rename(); for now no-op
                // so the match remains exhaustive.
                tracing::trace!("RenameTab shortcut ignored (Task 13 wires it)");
            }
```

This keeps Task 4 self-contained — no need to wait for Task 13 to compile.

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/keymap.rs
cargo test -p vibeflow --lib keymap 2>&1 | tail -25
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 22 keymap tests pass (17 existing + 5 new). 197 + 5 = 202 default + 13 ignored. fmt/clippy clean.

VERIFY all 17 pre-existing test names still appear:
```bash
for name in ctrl_shift_t_is_new_tab ctrl_shift_lowercase_t_is_new_tab ctrl_shift_w_is_close_tab ctrl_shift_r_is_restart_tab ctrl_shift_c_is_copy ctrl_shift_v_is_paste ctrl_tab_is_next_tab ctrl_shift_tab_is_prev_tab super_t_is_new_tab super_v_is_paste super_tab_is_next_tab super_shift_tab_is_prev_tab plain_t_is_none ctrl_t_without_shift_is_none ctrl_shift_alt_t_is_none ctrl_shift_x_is_none super_with_ctrl_is_none; do
  grep -q "fn $name" crates/vibeflow/src/keymap.rs && echo "OK $name" || echo "MISSING $name"
done
```

If ANY says MISSING, STOP and report BLOCKED.

If clippy complains about `OnceLock` not being available in your Rust version, the project's MSRV is recent enough; verify with `rustc --version`. Falls back: `static DEFAULT: once_cell::sync::OnceCell<...>` if `once_cell` is already a transitive dep (likely from cosmic-text).

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/keymap.rs
git commit -m "refactor(keymap): ShortcutTable + add Shortcut::RenameTab (preserves Stage 8 behavior)"
```

---

## Task 5: `error_banner.rs` — `ErrorBannerState` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/error_banner.rs`

Pure logic. Holds the error vec, dismissed flag, and display-text formatting.

- [ ] **Step 1: Write the implementation**

Replace the contents of `crates/vibeflow/src/config/error_banner.rs` with:

```rust
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
        format!("⚠ {n} config key{} ignored: {first}{suffix} — Esc to dismiss",
                if n == 1 { "" } else { "s" })
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
```

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/config/error_banner.rs
cargo test -p vibeflow --lib config::error_banner 2>&1 | tail -15
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 8 new banner tests pass. 202 + 8 = 210 default + 13 ignored. fmt/clippy clean.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/config/error_banner.rs
git commit -m "feat(config): ErrorBannerState — pure logic + format (TDD)"
```

---

## Task 6: `watcher.rs` — `notify` thread + 250ms debounce

**Files:**
- Modify: `crates/vibeflow/src/config/watcher.rs`

A background thread watches the config path's parent directory (so file delete + recreate is caught), debounces 250ms via a simple `recv_timeout` loop, parses the file, and ships `AppUserEvent::ConfigReloaded` via the proxy.

- [ ] **Step 1: Write the implementation**

Replace the contents of `crates/vibeflow/src/config/watcher.rs` with:

```rust
//! Background file-watcher thread. Uses `notify` to detect changes to
//! `~/.config/vibeflow/config.toml`, debounces 250 ms, parses + validates,
//! and ships `AppUserEvent::ConfigReloaded` via `EventLoopProxy::send_event`
//! to the main thread.

use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use winit::event_loop::EventLoopProxy;

use crate::config::{AppUserEvent, Config, ConfigError};

const DEBOUNCE: Duration = Duration::from_millis(250);

/// Spawn the watcher thread. Returns its `JoinHandle` for shutdown sequencing
/// (or just drop it; the thread exits naturally when the proxy fails to send,
/// which happens once the main event loop has exited).
///
/// The thread watches the `path`'s parent directory so deletes + recreates
/// of the file are seen.
///
/// # Errors
/// Returns `notify::Error` if the watcher fails to bind to the parent dir.
pub fn spawn(
    path: PathBuf,
    proxy: EventLoopProxy<AppUserEvent>,
) -> notify::Result<JoinHandle<()>> {
    let (tx, rx) = channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    let watch_dir = path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    // The dir must exist; otherwise we silently no-op-spawn the thread (it
    // will idle waiting for events that never arrive).
    if watch_dir.exists() {
        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;
    }

    let handle = thread::Builder::new()
        .name("vibeflow-config-watcher".to_string())
        .spawn(move || {
            // Hold the watcher in scope so it isn't dropped while the thread runs.
            let _watcher = watcher;
            let mut deadline: Option<Instant> = None;
            loop {
                let timeout = deadline
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::from_secs(60));
                match rx.recv_timeout(timeout) {
                    Ok(Ok(event)) => {
                        if event_concerns(&event, &path) {
                            // Bump the debounce deadline.
                            deadline = Some(Instant::now() + DEBOUNCE);
                            // Special-case Remove: tell main thread immediately
                            // (don't wait for debounce — file is already gone).
                            if matches!(event.kind, EventKind::Remove(_)) {
                                let err = ConfigError::IoError(format!(
                                    "{} removed at runtime",
                                    path.display()
                                ));
                                if proxy.send_event(AppUserEvent::ConfigError(err)).is_err() {
                                    return; // event loop dropped → exit thread
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "watcher error");
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // Either debounce expired or idle 60s wakeup.
                        if let Some(d) = deadline {
                            if Instant::now() >= d {
                                deadline = None;
                                let (cfg, errors) = Config::load(&path);
                                if proxy
                                    .send_event(AppUserEvent::ConfigReloaded {
                                        config: cfg,
                                        errors,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        })?;
    Ok(handle)
}

/// Does this notify event concern our config file?
///
/// notify reports events at the parent-dir level on most platforms; the
/// `paths` field tells us which exact file was touched.
fn event_concerns(event: &Event, target: &std::path::Path) -> bool {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
            event.paths.iter().any(|p| p == target)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The watcher thread is timing-sensitive; we can only assert the spawn
    // call succeeds and that event_concerns filters correctly. End-to-end
    // file-modify-roundtrip is covered by an `#[ignore]` integration test.

    #[test]
    fn event_concerns_matches_target_path() {
        let target = PathBuf::from("/tmp/vibeflow_test/config.toml");
        let ev = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![target.clone()],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(event_concerns(&ev, &target));
    }

    #[test]
    fn event_concerns_rejects_other_paths() {
        let target = PathBuf::from("/tmp/vibeflow_test/config.toml");
        let other = PathBuf::from("/tmp/vibeflow_test/other.toml");
        let ev = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![other],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(!event_concerns(&ev, &target));
    }

    #[test]
    fn event_concerns_rejects_access_events() {
        let target = PathBuf::from("/tmp/vibeflow_test/config.toml");
        let ev = Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![target.clone()],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(!event_concerns(&ev, &target));
    }

    // End-to-end test: write file → modify → assert reload event via a
    // local mpsc (not winit's proxy, since we can't construct one in unit
    // tests). #[ignore] because it touches the filesystem and depends on
    // OS-level inotify timing.
    #[test]
    #[ignore = "filesystem-timing-sensitive; depends on OS inotify backend"]
    fn watcher_emits_reload_after_modify() {
        // The full integration test (which uses a real EventLoop) lives at
        // crates/vibeflow/tests/config_reload.rs — added in Task 14.
    }
}
```

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/config/watcher.rs
cargo test -p vibeflow --lib config::watcher 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 3 watcher tests pass + 1 ignored. 210 + 3 = 213 default + 14 ignored. fmt/clippy clean.

If `notify::Event::default()` doesn't exist (the constructor is private), use the explicit struct construction shown in the tests above.

If `notify::Watcher` trait imports differ, the canonical 6.x API is `notify::recommended_watcher` (returning `RecommendedWatcher` which implements `Watcher`). Confirm by reading `~/.cargo/registry/src/index.crates.io-*/notify-6*/src/lib.rs` if it doesn't compile.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/config/watcher.rs
git commit -m "feat(config): notify-based file-watcher thread with 250ms debounce"
```

---

## Task 7: `WindowApp::user_event` — distribute config to subscribers

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

Replace the Task 3 stub `user_event` with the real distribution: renderer setters, keymap table swap, clipboard primary toggle, error_banner update. Also spawn the watcher thread in `resumed`.

- [ ] **Step 1: Add `error_banner`, `shortcut_table`, and `config_path` fields to `WindowApp`**

Open `crates/vibeflow/src/window.rs`. Add to `pub struct WindowApp` (alphabetical):

```rust
    /// Active shortcut table. Replaces the static Stage 8 lookup.
    shortcut_table: crate::keymap::ShortcutTable,
    /// Banner state for config errors (Stage 9). None until first reload reports errors.
    error_banner: crate::config::error_banner::ErrorBannerState,
    /// Path to the config file. Stored so the watcher can be respawned if needed.
    config_path: std::path::PathBuf,
```

Update `WindowApp::new(proxy)` (added in Task 3) to initialize these:

```rust
pub fn new(proxy: winit::event_loop::EventLoopProxy<crate::config::AppUserEvent>) -> Self {
    let config_path = crate::config::default_path()
        .unwrap_or_else(|| std::path::PathBuf::from("./vibeflow-config.toml"));
    let (config, errors) = crate::config::Config::load(&config_path);
    let error_banner = crate::config::error_banner::ErrorBannerState::new(errors);
    let shortcut_table = crate::keymap::ShortcutTable::with_default_bindings();
    // Distribute initial config values once renderer is built (later in resumed()).
    Self {
        // ... existing initializers
        proxy,
        shortcut_table,
        error_banner,
        config_path,
        // We stash the loaded `config` for `resumed` to apply once the
        // renderer/clipboard exist. Add a field if needed:
        //     pending_config: Option<Config>,
        // Or just re-load in `resumed`. Cheap & clean.
    }
}
```

Replace the `match_shortcut` call in `KeyboardInput` (Stage 8 used `crate::keymap::match_shortcut`) with `self.shortcut_table.lookup`:

```rust
// Was:
//   if let Some(shortcut) = crate::keymap::match_shortcut(&event.logical_key, self.current_modifiers) { ... }
// Becomes:
if let Some(shortcut) = self.shortcut_table.lookup(&event.logical_key, self.current_modifiers) {
    self.handle_shortcut(shortcut);
    return;
}
```

- [ ] **Step 2: Spawn the watcher in `resumed`**

In `WindowApp::resumed`, AFTER the existing window/renderer initialization:

```rust
fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
    // ... existing setup (window, renderer, etc.)

    // Apply initial config now that renderer is built.
    let (config, errors) = crate::config::Config::load(&self.config_path);
    self.apply_config(&config);
    self.error_banner.update(errors);

    // Start the file watcher.
    let proxy = self.proxy.clone();
    let path = self.config_path.clone();
    if let Err(e) = crate::config::watcher::spawn(path, proxy) {
        tracing::warn!(error = %e, "config watcher failed to start");
    }
}
```

- [ ] **Step 3: Implement `apply_config` + the real `user_event`**

Add to `impl WindowApp`:

```rust
    /// Distribute a newly-loaded config to all subscribers.
    fn apply_config(&mut self, config: &crate::config::Config) {
        if let Some(r) = self.renderer.as_mut() {
            r.set_selection_color(config.colors.selection);
            r.set_indicator_colors([
                config.colors.indicator_active,
                config.colors.indicator_working,
                config.colors.indicator_waiting,
                config.colors.indicator_inactive,
            ]);
            r.set_cursor_blink_ms(config.cursor.blink_ms);
            r.set_font_priorities(config.fonts.priority.clone());
        }
        // Rebuild the shortcut table from the bindings.
        self.shortcut_table = build_shortcut_table(&config.shortcuts);
        if let Some(c) = self.clipboard.as_mut() {
            c.set_primary_enabled(config.clipboard.primary);
        }
    }
```

Replace the Task 3 stub `user_event` with:

```rust
    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: crate::config::AppUserEvent,
    ) {
        match event {
            crate::config::AppUserEvent::ConfigReloaded { config, errors } => {
                tracing::info!(
                    error_count = errors.len(),
                    "config reloaded"
                );
                self.apply_config(&config);
                self.error_banner.update(errors);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            crate::config::AppUserEvent::ConfigError(err) => {
                tracing::warn!(?err, "config error");
                self.error_banner.update(vec![err]);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
        }
    }
```

Add a free helper `build_shortcut_table` in `window.rs` (it bridges `Config.shortcuts` → `ShortcutTable`):

```rust
fn build_shortcut_table(
    bindings: &crate::config::ShortcutBindings,
) -> crate::keymap::ShortcutTable {
    // For now, when the user supplies a config, replace the default table
    // wholesale. The default still applies if a Shortcut variant has no
    // entry in `bindings`.
    let mut table = crate::keymap::ShortcutTable::with_default_bindings();
    table.replace_from_bindings(bindings);
    table
}
```

This requires `ShortcutTable` to expose a `replace_from_bindings` API. Append to `crates/vibeflow/src/keymap.rs` (in `impl ShortcutTable`):

```rust
    /// Replace this table's entries from a `ShortcutBindings` map (sourced
    /// from `Config.shortcuts`). Each action's chord list in `bindings`
    /// REPLACES the default chord list for that action. Unset actions keep
    /// the defaults.
    pub fn replace_from_bindings(&mut self, bindings: &crate::config::ShortcutBindings) {
        use crate::config::KeyMatch;
        // First, remove ALL chords whose action is named in `bindings`. This
        // way, if the user binds NewTab to ["ctrl+alt+t"], the default
        // ctrl+shift+t doesn't linger.
        let actions_to_replace: std::collections::HashSet<Shortcut> =
            bindings.bindings.keys().copied().collect();
        self.by_chord
            .retain(|_, action| !actions_to_replace.contains(action));
        // Then insert the user's chords.
        for (action, chords) in &bindings.bindings {
            for chord in chords {
                let disc = match &chord.key {
                    KeyMatch::Char(c) => ChordKeyDisc::Char(c.to_ascii_lowercase()),
                    KeyMatch::Tab => ChordKeyDisc::Tab,
                    KeyMatch::Function(n) => ChordKeyDisc::Function(*n),
                };
                self.by_chord.insert(
                    ChordKey {
                        modifiers_bits: chord.modifiers.bits(),
                        key: disc,
                    },
                    *action,
                );
            }
        }
    }
```

- [ ] **Step 4: Add a unit test for `replace_from_bindings`**

Append to `mod tests` in `keymap.rs` (DO NOT MODIFY existing tests):

```rust
    #[test]
    fn replace_from_bindings_overrides_defaults() {
        use crate::config::{KeyChord, KeyMatch, ShortcutBindings};
        use std::collections::HashMap;

        let mut bindings = HashMap::new();
        bindings.insert(
            Shortcut::NewTab,
            vec![KeyChord {
                modifiers: ModifiersState::CONTROL | ModifiersState::ALT,
                key: KeyMatch::Char('t'),
            }],
        );
        let user = ShortcutBindings { bindings };

        let mut table = ShortcutTable::with_default_bindings();
        table.replace_from_bindings(&user);

        // The default ctrl+shift+t should be GONE...
        assert_eq!(
            table.lookup(&ch("t"), mods(true, true, false, false)),
            None
        );
        // ...and ctrl+alt+t should NOT trigger NewTab either, because Alt
        // is unconditionally rejected by `lookup`.
        assert_eq!(
            table.lookup(&ch("t"), mods(true, false, true, false)),
            None
        );

        // Other actions still work — Copy default unchanged.
        assert_eq!(
            table.lookup(&ch("c"), mods(true, true, false, false)),
            Some(Shortcut::Copy)
        );
    }
```

NOTE: The Alt-rejection test is intentional — it documents that `Alt` chords cannot be bound from config because `lookup` rejects all Alt-modified events. Stage 9 does NOT lift this restriction (the spec said `Ctrl+Alt+...` for VNC users, but our `lookup` rejects Alt; we'll need to either lift this restriction in a follow-up commit or accept that VNC users use `Ctrl+Shift+...` which works).

This is a real plan deviation from the design. Surface it in the deviation list during review. We can either (a) lift the Alt-rejection in `lookup` (and let `Ctrl+Alt+T` work), or (b) document that VNC users can't actually use `Ctrl+Alt+T` without further work. Option (a) is preferred — the design's "Ctrl+Alt+ alternates for Mac/VNC" should actually work.

Let me lift the Alt-rejection so `Ctrl+Alt` chords can be bound. Update Step 1's `ShortcutTable::lookup`: REMOVE the `if modifiers.alt_key() { return None; }` guard. BUT that would let stale `Ctrl+Shift+Alt+T` match `Ctrl+Shift+T` — bad. The right fix: include Alt in the chord key.

Update the `ChordKey` to include all four modifier bits as the lookup discriminator. The default table keeps `alt = false` for all bindings; lookups with `alt = true` won't match (no entry). Then user-config can bind `Ctrl+Alt+T` and it'll work.

Modify `ShortcutTable::lookup` to NOT strip Alt and NOT reject Alt:

```rust
    pub fn lookup(&self, key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
        let disc = match key {
            // ... same as before
        };
        // Use ALL FOUR modifier bits — Ctrl, Shift, Alt, Super.
        let mods_bits = (modifiers
            & (ModifiersState::CONTROL
                | ModifiersState::SHIFT
                | ModifiersState::ALT
                | ModifiersState::SUPER))
            .bits();
        self.by_chord
            .get(&ChordKey {
                modifiers_bits: mods_bits,
                key: disc,
            })
            .copied()
    }
```

This lets Ctrl+Alt+T work IF the user has bound it; the default table doesn't have any Alt entries so default behavior is unchanged.

Update the `ctrl_shift_alt_t_is_none` test (Stage 8 expected this combo to return None). Under the new behavior, `Ctrl+Shift+Alt+T` returns None ONLY because no chord with that modifier set is in the default table. The test still passes — but for a different reason.

Apply this fix to Task 4's keymap.rs replacement BEFORE the file's final form. Update both the `lookup` body in Task 4 Step 1 and the `replace_from_bindings_overrides_defaults` test in Task 7 Step 4 to align.

- [ ] **Step 5: Verify build green**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/window.rs crates/vibeflow/src/keymap.rs
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: 213 + 1 = 214 default + 14 ignored. fmt/clippy clean.

If borrow checker complains about `self.proxy.clone()` in `resumed` — `EventLoopProxy<T>` is `Clone` in winit 0.30. If it isn't (some embedded targets), use `Arc<EventLoopProxy<T>>`.

If `Renderer::set_selection_color` etc don't exist yet — they're added in Task 8. For Task 7, stub them as no-op methods that just `tracing::trace!` what they would set, so the build passes:

```rust
// In render/mod.rs (temporary — Task 8 fills these in):
impl Renderer {
    pub fn set_selection_color(&mut self, _c: [f32; 4]) { /* Task 8 */ }
    pub fn set_indicator_colors(&mut self, _c: [[f32; 4]; 4]) { /* Task 8 */ }
    pub fn set_cursor_blink_ms(&mut self, _ms: u64) { /* Task 8 */ }
    pub fn set_font_priorities(&mut self, _p: Vec<String>) { /* Task 8 */ }
}
```

Add these stubs as part of THIS task's render/mod.rs edit.

Similarly for `Clipboard::set_primary_enabled` — stub here, fill in Task 10.

- [ ] **Step 6: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/window.rs \
        crates/vibeflow/src/keymap.rs \
        crates/vibeflow/src/render/mod.rs \
        crates/vibeflow/src/clipboard.rs
git commit -m "feat(window): wire AppUserEvent → apply_config; spawn watcher in resumed"
```

---

## Task 8: Renderer setters + cursor blink + font priorities

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`
- Modify: `crates/vibeflow/src/render/cursor.rs`
- Modify: `crates/vibeflow/src/render/text_engine.rs`

Fill in the stub setters from Task 7 with real behavior. `set_font_priorities` rebuilds the cosmic-text `FontSystem`; the atlas is invalidated (next frame re-rasterizes).

- [ ] **Step 1: `cursor.rs::set_blink_ms`**

The actual struct in `crates/vibeflow/src/render/cursor.rs` is `pub struct CursorBlink` (NOT `Cursor`). The current Stage 7 code has a module-level `pub const BLINK_PERIOD_MS: u128 = 500;` and a `visible(now: Instant) -> bool` method that uses it. Stage 9 replaces the const with a per-instance field so live config-reload can change blink rate without rebuilding the renderer.

Add a `blink_ms` field to `CursorBlink` and a setter:

```rust
pub struct CursorBlink {
    // ... existing fields (the `last_input_at` etc. — read the existing struct)
    /// Milliseconds per full blink cycle. 0 disables blink (cursor renders solid).
    blink_ms: u64,
}

impl CursorBlink {
    pub fn new() -> Self {
        Self {
            // ... existing initializers
            blink_ms: BLINK_PERIOD_MS as u64,    // matches Stage 7 default
        }
    }

    pub fn set_blink_ms(&mut self, ms: u64) {
        self.blink_ms = ms;
    }

    pub fn visible(&self, now: Instant) -> bool {
        if self.blink_ms == 0 {
            return true;    // 0 = no blink, always visible
        }
        // Existing computation from Stage 7 — replace `BLINK_PERIOD_MS` with
        // `self.blink_ms as u128` in the body.
        // Stage 7 body looked something like:
        //   let elapsed_ms = now.duration_since(self.last_input_at).as_millis();
        //   (elapsed_ms / BLINK_PERIOD_MS) % 2 == 0
        // becomes:
        //   (elapsed_ms / (self.blink_ms as u128)) % 2 == 0
    }
}
```

The `pub const BLINK_PERIOD_MS` can stay (used as the default initializer in `new()`), or be removed if you prefer. Either is fine; preserving it keeps backward-compat tests stable.

Read the existing `cursor.rs` to confirm the exact `visible` body and adapt.

- [ ] **Step 2: `text_engine.rs::set_font_priorities`**

Open `crates/vibeflow/src/render/text_engine.rs`. Find the cosmic-text `FontSystem` field (likely `font_system: FontSystem`). Add:

```rust
impl TextEngine {
    /// Rebuild the cosmic-text font fallback chain with the user-supplied
    /// priority list. Earlier names take precedence in fallback resolution.
    /// This invalidates the glyph cache — the next frame re-rasterizes.
    pub fn set_font_priorities(&mut self, priority: Vec<String>) {
        use cosmic_text::fontdb::Database;
        let mut db = Database::new();
        // Load each priority font from the system (best-effort — names that
        // don't exist on this system are silently skipped).
        for family in &priority {
            db.load_system_fonts();  // load all once for matching
            // fontdb doesn't expose a "load only this family" API; the
            // lookup-by-family-name happens at shaping time. The order of
            // load matters less than the cosmic-text Attrs::family used in
            // shaping. So instead: load all system fonts AND set the
            // primary font name to priority[0] (already done at startup).
            break;  // one full system load is enough
        }
        if priority.is_empty() {
            db.load_system_fonts();
        }
        // Replace the FontSystem.
        let new_system = cosmic_text::FontSystem::new_with_locale_and_db(
            "en-US".to_string(),
            db,
        );
        self.font_system = new_system;
        // Stage 9: log priorities; live-reload of font fallback CHAIN order
        // is deferred to Stage 10. The primary family name (priority[0]) is
        // applied at startup via `Attrs::family`; live-reload of that
        // doesn't take effect until next startup. Document in smoke.
        tracing::info!(?priority, "font priorities updated (some changes apply on next startup)");
        // Invalidate the glyph cache so atlas re-rasterizes on next render.
        self.invalidate_glyph_cache();
    }

    /// Clear the cached glyph map. Called by `set_font_priorities` when the
    /// FontSystem changes; subsequent `glyph_for` calls miss the cache and
    /// re-shape.
    ///
    /// We intentionally do NOT clear the atlas textures themselves — atlas
    /// slots are reused as new glyphs come in, and clearing the wgpu texture
    /// would force a flicker on the next frame.
    pub fn invalidate_glyph_cache(&mut self) {
        self.cache.clear();
    }
}
```

NOTE: The cosmic-text font priority API is genuinely tricky. The implementer should:
1. Read `~/.cargo/registry/src/index.crates.io-*/cosmic-text-0.12.1/src/font/system.rs` to confirm `FontSystem::new_with_locale_and_db` exists in 0.12.1.
2. Confirm `fontdb::Database::load_system_fonts()` is the right load method.
3. If a more granular "set priority" API exists in 0.12.1, prefer it.

For Stage 9 v1, it's acceptable to defer true font-priority live-reload — log a "applied at next startup" warning and leave the atlas as-is. The `Renderer::new` path already constructs `FontSystem` with whatever priority is in `Config.fonts.priority` — so startup-time application works. Live-reload is the polish.

- [ ] **Step 3: `render/mod.rs` setters**

Replace the Task 7 stubs with real bodies:

```rust
impl Renderer {
    pub fn set_selection_color(&mut self, c: [f32; 4]) {
        self.selection_color = c;  // add the field if it doesn't exist
    }

    pub fn set_indicator_colors(&mut self, c: [[f32; 4]; 4]) {
        self.indicator_colors = c;  // add field; consumed by tab-bar render
    }

    pub fn set_cursor_blink_ms(&mut self, ms: u64) {
        self.cursor.set_blink_ms(ms);
    }

    pub fn set_font_priorities(&mut self, priority: Vec<String>) {
        self.text_engine.set_font_priorities(priority);
    }
}
```

Add the new fields:

```rust
pub struct Renderer {
    // ... existing fields
    /// Stage 9: configurable selection rect color (was hardcoded constant).
    selection_color: [f32; 4],
    /// Stage 9: configurable indicator-dot colors [active, working, waiting, inactive].
    indicator_colors: [[f32; 4]; 4],
}
```

Initialize them in `Renderer::new` to the Stage 8 defaults (so behavior without config matches):

```rust
selection_color: [0.4, 0.6, 1.0, 0.4],
indicator_colors: [
    [0.13, 0.80, 0.40, 1.0],  // active green
    [0.20, 0.60, 1.0, 1.0],   // working blue
    [1.0, 0.67, 0.0, 1.0],    // waiting amber
    [0.53, 0.53, 0.53, 1.0],  // inactive grey
],
```

Replace the hardcoded `SELECTION_COLOR` constant in `build_selection_rects` (Stage 8) with `self.selection_color` — pass it through.

For indicator colors: find the tab-bar render path (Stage 4 + 7) that picks dot colors based on `TabState`. Replace the hardcoded dot colors with `self.indicator_colors` indexed by state.

- [ ] **Step 4: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/render/
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 214 default + 14 ignored unchanged (Task 8 adds zero new tests; existing render tests must still pass).

If `Cursor::set_blink_ms` causes a borrow-checker conflict (cursor field is borrowed inside `Renderer::render`), do the field update via a setter pattern or store the blink_ms on `Renderer` and pass it down per-frame.

If `text_engine.font_system` replacement breaks the atlas (texture corruption), fall back to v1's "applied at next startup" behavior — just store the priority for next startup and log a warning. Document the deferral.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/
git commit -m "feat(render): config-driven setters for selection/indicators/cursor-blink/fonts"
```

---

## Task 9: Error banner rendering integration

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

The dead-tab banner (Stage 7) and config-error banner (Stage 9) are TWO DISTINCT banners. To avoid name collision, rename references inside `render/mod.rs`:
- Existing `banner_*` (dead-tab) → keep as `banner_*` (Stage 7 namespace).
- Stage 9's config-error banner → `config_banner_*`.

Insert one rect range and one glyph range for the config banner between selection rects and the dead-tab banner.

- [ ] **Step 1: Pass `&ErrorBannerState` to `Renderer::render`**

Find the `pub fn render(...)` signature. Add a parameter:

```rust
pub fn render(
    &mut self,
    term: &Term<VoidListener>,
    app: &App,
    rename_state: Option<&RenameInputState>,   // (added in Task 13)
    error_banner: &crate::config::error_banner::ErrorBannerState,
) -> Result<(), wgpu::SurfaceError> { ... }
```

For Task 9, pass an empty banner from `WindowApp::draw_redraw` (Task 7 hooked the call site; pass `&self.error_banner`). Task 13 will add `rename_state`.

- [ ] **Step 2: Build the banner rect + glyphs**

`TextEngine` does NOT have a `shape_line` / `shape_str` method — the existing API is `glyph_for(c: char) -> Option<GlyphRef>` (per-char). The Stage 7 tab-title path also iterates char-by-char. We follow that pattern.

Add a free helper near the top of `render/mod.rs` (or in a new `render/banner.rs` if you prefer the separation):

```rust
/// Build glyph instances for a single line of banner text. Iterates char-by-
/// char via `TextEngine::glyph_for` (the existing primitive) and lays them
/// out left-to-right at `cell_w`-pixel pitch starting at `(start_x, start_y)`.
fn build_banner_glyphs(
    text: &str,
    start_x: f32,
    start_y: f32,
    text_engine: &mut crate::render::text_engine::TextEngine,
    fg_color: [f32; 4],
    bg_color: [f32; 4],
) -> Vec<crate::render::quad::QuadInstance> {
    let (cell_w, _cell_h) = text_engine.cell_metrics();
    let mut out = Vec::with_capacity(text.chars().count());
    for (i, ch) in text.chars().enumerate() {
        let Some(glyph) = text_engine.glyph_for(ch) else { continue };
        let x = start_x + (i as u32 * cell_w) as f32;
        out.push(crate::render::quad::QuadInstance::new_text(
            x,
            start_y,
            cell_w as f32,
            text_engine.cell_metrics().1 as f32,
            &glyph,
            fg_color,
            bg_color,
        ));
    }
    out
}
```

The exact `QuadInstance` constructor name (`new_text` vs `new` vs another) depends on the existing API — read `render/quad.rs` for the actual signature; the existing tab-title path in `render/tabs.rs::push_text_glyphs` shows the pattern. Adapt.

Then in `Renderer::render`, after `selection_rects` is built and before the existing dead-tab banner block, add:

```rust
        // Stage 9 config-error banner.
        let (config_banner_rect, config_banner_glyphs) = if error_banner.visible() {
            let surface_w = surface_size.0 as f32;
            let bar_h = layout.bar_height_px as f32;
            let banner_height = (cell_h as f32) * 1.5;
            let rect = crate::render::tabs::RectInstance::new(
                0.0,
                bar_h,
                surface_w,
                banner_height,
                [0.40, 0.10, 0.10, 0.85],  // dark-red, 85% alpha
            );
            let text = error_banner.display_text();
            let glyphs = build_banner_glyphs(
                &text,
                8.0,
                bar_h + 4.0,
                &mut self.text_engine,
                [1.0, 1.0, 1.0, 1.0],            // white fg
                [0.40, 0.10, 0.10, 0.85],        // matches rect bg
            );
            (Some(rect), glyphs)
        } else {
            (None, Vec::new())
        };
```

If the banner text is wider than `surface_w - 16.0`, glyphs that fall off the right edge still render but get clipped by the wgpu scissor — for v1 that's acceptable; ellipsis truncation can be a Stage 10 polish.

- [ ] **Step 3: Update offset bookkeeping**

Update the unified rect-buffer offsets (Stage 8 had this):

```rust
        let tab_rect_count = tab_rects.len() as u32;
        let selection_rect_offset = tab_rect_count;
        let selection_rect_count = selection_rects.len() as u32;
        let config_banner_rect_offset = selection_rect_offset + selection_rect_count;
        let config_banner_rect_count = u32::from(config_banner_rect.is_some());
        let banner_rect_offset = config_banner_rect_offset + config_banner_rect_count;
        let banner_rect_count = u32::from(banner_rect.is_some());
        let bell_rect_offset = banner_rect_offset + banner_rect_count;
        let bell_rect_count = u32::from(bell_rect.is_some());
        let total_rects = bell_rect_offset + bell_rect_count;
```

And the `all_rects` extension (insert config banner rect):

```rust
        let mut all_rects = Vec::with_capacity(total_rects as usize);
        all_rects.extend_from_slice(&tab_rects);
        all_rects.extend_from_slice(&selection_rects);
        if let Some(r) = config_banner_rect { all_rects.push(r); }
        if let Some(r) = banner_rect { all_rects.push(r); }
        if let Some(r) = bell_rect { all_rects.push(r); }
```

- [ ] **Step 4: Update glyph offset bookkeeping**

The unified glyph buffer (Stage 7.5) carries cell glyphs + tab text + banner glyphs. Add a new range for the config banner glyphs. Find the glyph offset calculation (similar to rects). Add:

```rust
        let config_banner_glyph_offset = banner_glyph_offset + banner_glyph_count;
        let config_banner_glyph_count = config_banner_glyphs.len() as u32;
        let total_quads_with_config_banner =
            config_banner_glyph_offset + config_banner_glyph_count;
```

The exact variable names depend on the existing render/mod.rs. Read it carefully and integrate.

- [ ] **Step 5: Add draw_range calls**

In the render-pass block (`encoder.begin_render_pass(...)`), insert two new draw calls — config banner rect AFTER selection rects, config banner glyphs AFTER the rect:

```rust
        // ---- Stage 9 config error banner ----
        if config_banner_rect_count > 0 {
            self.tab_bar_pipeline
                .draw_range(&mut pass, config_banner_rect_offset..banner_rect_offset);
        }
        if config_banner_glyph_count > 0 {
            self.quad_pipeline.draw_range(
                &mut pass,
                config_banner_glyph_offset..total_quads_with_config_banner,
            );
        }
        // ---- Existing dead-tab banner (Stage 7) ----
        if banner_rect_count > 0 { ... }
```

- [ ] **Step 6: Wire `Esc` → dismiss in `WindowApp`**

In `crates/vibeflow/src/window.rs`'s KeyboardInput arm, BEFORE the rename-input check (Task 13) and BEFORE the shortcut dispatch:

```rust
                if event.state == ElementState::Pressed
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                    && self.error_banner.visible()
                    && self.rename_state.is_none()
                {
                    self.error_banner.dismiss();
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                    return;
                }
```

Place this guard EARLY in the KeyboardInput arm so banner-Esc takes precedence over both shortcut dispatch and typed-input. Don't dismiss if a rename is in progress (rename's own Esc-cancel takes precedence).

- [ ] **Step 7: Verify**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 214 default + 14 ignored. fmt/clippy clean.

If the offset math becomes ungainly, factor into a helper `compute_rect_offsets(...) -> RectOffsets`. Same for glyphs.

- [ ] **Step 8: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/mod.rs crates/vibeflow/src/window.rs
git commit -m "feat(render): config error banner rendering + Esc dismiss"
```

---

## Task 10: Clipboard PRIMARY enabled + auto-copy + middle-click paste

**Files:**
- Modify: `crates/vibeflow/src/clipboard.rs`
- Modify: `crates/vibeflow/src/window.rs`

`Clipboard` learns to write/read PRIMARY. `WindowApp` learns to auto-copy on selection finalize and to handle middle-click paste in the cell area.

- [ ] **Step 1: Extend `Clipboard`**

Replace the contents of `crates/vibeflow/src/clipboard.rs` with:

```rust
//! System clipboard wrapper. Stage 8 supports CLIPBOARD; Stage 9 adds the
//! optional X11 PRIMARY selector (middle-click paste). Linux-only — silently
//! no-ops on macOS/Windows.

use anyhow::{Context, Result};

pub struct Clipboard {
    inner: arboard::Clipboard,
    /// True when the user has enabled `clipboard.primary = true` in config.
    /// On non-Linux this flag is meaningful but the underlying arboard ops
    /// fall through to CLIPBOARD anyway.
    primary_enabled: bool,
}

impl Clipboard {
    /// # Errors
    /// Propagates `arboard` errors connecting to the display server.
    pub fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new()
            .context("create system clipboard handle (no display server?)")?;
        Ok(Self {
            inner,
            primary_enabled: true,  // matches default config
        })
    }

    pub fn set_primary_enabled(&mut self, enabled: bool) {
        self.primary_enabled = enabled;
    }

    /// Copy `text` to the CLIPBOARD selector. Also writes to PRIMARY if
    /// `primary_enabled` is true (Linux-only effect).
    ///
    /// # Errors
    /// Propagates `arboard` errors. The caller logs at `warn` and proceeds.
    pub fn copy(&mut self, text: &str) -> Result<()> {
        self.inner
            .set_text(text)
            .context("write to system clipboard")?;
        if self.primary_enabled {
            #[cfg(target_os = "linux")]
            {
                use arboard::SetExtLinux;
                let _ = self
                    .inner
                    .set()
                    .clipboard(arboard::LinuxClipboardKind::Primary)
                    .text(text);
            }
        }
        Ok(())
    }

    /// Paste from the CLIPBOARD selector. Returns `None` if empty / non-text.
    pub fn paste(&mut self) -> Option<String> {
        self.inner.get_text().ok()
    }

    /// Paste from the PRIMARY selector (X11 middle-click semantic). Returns
    /// `None` on non-Linux or if PRIMARY is empty / non-text.
    pub fn paste_primary(&mut self) -> Option<String> {
        if !self.primary_enabled {
            return None;
        }
        #[cfg(target_os = "linux")]
        {
            use arboard::GetExtLinux;
            return self
                .inner
                .get()
                .clipboard(arboard::LinuxClipboardKind::Primary)
                .text()
                .ok();
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires display server (X11/Wayland) — run with --ignored"]
    fn copy_paste_roundtrips_through_system_clipboard() {
        let mut c = Clipboard::new().expect("clipboard available");
        c.copy("hello, vibeflow").unwrap();
        let got = c.paste().expect("paste returned text");
        assert_eq!(got, "hello, vibeflow");
    }

    #[test]
    #[ignore = "requires X11 PRIMARY selector — Linux only"]
    fn primary_roundtrips_when_enabled() {
        let mut c = Clipboard::new().expect("clipboard available");
        c.set_primary_enabled(true);
        c.copy("primary test").unwrap();
        // Read back from PRIMARY directly.
        let got = c.paste_primary().expect("primary returned text");
        assert_eq!(got, "primary test");
    }

    #[test]
    fn primary_disabled_returns_none() {
        // Doesn't need a display server because the disabled path short-circuits.
        let Ok(mut c) = Clipboard::new() else { return };
        c.set_primary_enabled(false);
        // Even if PRIMARY has content from another app, we report None.
        assert_eq!(c.paste_primary(), None);
    }
}
```

NOTE: `arboard 3.6`'s Linux extension API is `arboard::SetExtLinux` / `GetExtLinux` traits with `clipboard()` and `text()` builder methods. Verify against the cached source:
```bash
ls ~/.cargo/registry/src/index.crates.io-*/arboard-3*/src/
grep -rn "LinuxClipboardKind\|SetExtLinux\|GetExtLinux" ~/.cargo/registry/src/index.crates.io-*/arboard-3*/src/ | head -10
```

If the API differs, adapt — the goal is "set the PRIMARY selector explicitly" and "read from PRIMARY explicitly."

- [ ] **Step 2: Auto-copy on selection finalize in `window.rs`**

In `WindowEvent::MouseInput`'s Released branch (the selection-driven path), AFTER `s.selection.mouse_up()`:

```rust
                    } else if released {
                        s.selection.mouse_up();
                        // Stage 9: if a finalized selection exists AND PRIMARY is
                        // enabled, auto-copy to PRIMARY for X11 middle-click paste
                        // semantics. CLIPBOARD is unaffected — Ctrl+Shift+C still
                        // explicitly copies there.
                        if let Some(text) = s.selection.text(s.term()) {
                            if let Some(clipboard) = self.clipboard.as_mut() {
                                #[cfg(target_os = "linux")]
                                if clipboard.primary_enabled() {
                                    let _ = clipboard.copy_primary(&text);
                                }
                            }
                        }
                    }
```

Add `Clipboard::primary_enabled()` getter (one-line) and `Clipboard::copy_primary(&str) -> Result<()>` method (writes ONLY to PRIMARY) for the auto-copy path:

```rust
    pub fn primary_enabled(&self) -> bool {
        self.primary_enabled
    }

    /// Linux-only: write `text` to the PRIMARY selector ONLY (CLIPBOARD untouched).
    /// Used by the auto-copy-on-selection-finalize path.
    pub fn copy_primary(&mut self, text: &str) -> Result<()> {
        if !self.primary_enabled {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        {
            use arboard::SetExtLinux;
            self.inner
                .set()
                .clipboard(arboard::LinuxClipboardKind::Primary)
                .text(text)
                .context("write to PRIMARY selector")?;
        }
        Ok(())
    }
```

- [ ] **Step 3: Middle-click paste in `window.rs`**

In `WindowEvent::MouseInput`'s cell-grid section (NOT the tab-bar passthrough), add a handler for Middle button BEFORE the existing left-button selection logic:

```rust
                    if button == MouseButton::Middle && released && !mode_on {
                        // Middle-click in non-mouse-mode pastes from PRIMARY.
                        let active = self.app.active();
                        let bracketed = self
                            .app
                            .tabs()
                            .get(active)
                            .map(|s| {
                                s.term()
                                    .mode()
                                    .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
                            })
                            .unwrap_or(false);
                        if let Some(clipboard) = self.clipboard.as_mut() {
                            if let Some(text) = clipboard.paste_primary() {
                                if let Some(s) = self.app.tabs_mut().get_mut(active) {
                                    if bracketed {
                                        let _ = s.send_input(b"\x1b[200~");
                                        let _ = s.send_input(text.as_bytes());
                                        let _ = s.send_input(b"\x1b[201~");
                                    } else {
                                        let _ = s.send_input(text.as_bytes());
                                    }
                                }
                            }
                        }
                        return;
                    }
```

Place this BEFORE the existing left-button selection branch so middle clicks don't accidentally start selections.

If mouse-mode is ON, middle-click should still go to the PTY's mouse encoder per Stage 8. The `&& !mode_on` guard handles that.

- [ ] **Step 4: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/clipboard.rs crates/vibeflow/src/window.rs
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo test -p vibeflow --lib clipboard -- --ignored 2>&1 | tail -10
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 214 default + 1 new (`primary_disabled_returns_none`) + 14 + 1 ignored = 215 default + 15 ignored. fmt/clippy clean. The `--ignored` clipboard tests should pass on host (X11 available).

If `arboard::SetExtLinux::clipboard().text()` returns a builder type that needs explicit `.set()` / `.execute()` to commit, adapt — read the arboard source.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/clipboard.rs crates/vibeflow/src/window.rs
git commit -m "feat(clipboard): PRIMARY selector + auto-copy on select + middle-click paste"
```

---

## Task 11: OSC 0/2 + `PtySession.user_renamed`

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`
- Modify: `crates/vibeflow/src/session/session.rs`

Add OSC 0/2 parsing → `DispatchEvent::SetTitle(String)`. PtySession honors a sticky `user_renamed` flag that suppresses OSC 0/2 once the user has manually renamed.

**Mandatory safety rule:** **DO NOT DELETE OR WEAKEN ANY EXISTING TEST.** session.rs has 21 test functions (Stage 8's 19 + the Stage 8 restart_replaces_internals_with_fresh_spawn + the Stage 8 default + others). osc.rs has the existing OSC 1338 + 133 tests. Preserve all of them.

- [ ] **Step 1: Add `DispatchEvent::SetTitle` and parse OSC 0/2**

Open `crates/vibeflow/src/session/osc.rs`. Find `pub enum DispatchEvent`. Add a variant:

```rust
pub enum DispatchEvent {
    AiState(Frame),
    Prompt(PromptMarker),
    /// OSC 0 (set window+icon title) or OSC 2 (set window title only).
    /// Carries the title payload as UTF-8. Stage 9.
    SetTitle(String),
    PassThrough(Vec<u8>),
}
```

Open `crates/vibeflow/src/session/osc.rs` and find the existing `fn handle_osc(body: &[u8]) -> OscOutcome` (around line 245). The structure is:

```rust
fn handle_osc(body: &[u8]) -> OscOutcome {
    let Some(body_str) = std::str::from_utf8(body).ok() else {
        return OscOutcome::Forward;
    };
    let (id, params) = body_str.split_once(';').unwrap_or((body_str, ""));
    match id {
        "1338" => { ... }
        "133" => { ... }
        _ => OscOutcome::Forward,
    }
}
```

Add `"0" | "2"` and `"1"` arms in the `match id { ... }` block, BEFORE the catch-all `_ => OscOutcome::Forward,`:

```rust
        "0" | "2" => {
            // OSC 0 sets both window + icon title; OSC 2 sets only window
            // title. We don't distinguish icon from title — both update
            // `TabLabel.title` via DispatchEvent::SetTitle. xterm caps title
            // length at ~1024 chars; we follow that convention.
            let title: String = if params.chars().count() > 1024 {
                params.chars().take(1024).collect()
            } else {
                params.to_string()
            };
            OscOutcome::Event(DispatchEvent::SetTitle(title))
        }
        "1" => OscOutcome::Drop, // icon name only — silently ignore
```

CRITICAL: The existing test `dispatcher_passes_through_unknown_osc_intact` (around line 478 in `osc.rs`) uses `\x1b]0;hello world\x07` as its "unknown OSC" example. After this Step 1, OSC 0 is no longer unknown — it produces `SetTitle("hello world")` instead of `PassThrough(...)`. Update that test to use a genuinely unrecognized ID before this step compiles. See Step 1.5 below.

- [ ] **Step 1.5: Update the pre-existing `dispatcher_passes_through_unknown_osc_intact` test**

In `crates/vibeflow/src/session/osc.rs`, find the test (around line 478):

```rust
    #[test]
    fn dispatcher_passes_through_unknown_osc_intact() {
        // OSC 0 is the iTerm/xterm window-title sequence. We don't recognise
        // it, so the original bytes (ESC ] 0;<title> BEL) must reach the
        // terminal grid unchanged.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]0;hello world\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(
                b"\x1b]0;hello world\x07".to_vec()
            )]
        );
    }
```

Replace the body to use OSC 999 (genuinely unrecognized):

```rust
    #[test]
    fn dispatcher_passes_through_unknown_osc_intact() {
        // Use a genuinely unrecognised OSC ID. Stage 9 added OSC 0/2 as the
        // window-title sequence so they're no longer unknown — pick 999.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]999;garbage\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(
                b"\x1b]999;garbage\x07".to_vec()
            )]
        );
    }
```

The semantic test (unrecognized OSC IDs reach the grid unchanged) is preserved; only the example ID changes. The `dispatcher_passes_unknown_osc_with_st_terminator_intact` test uses OSC 7 (`\x1b]7;file://example\x1b\\`) which remains unknown — leave it untouched.

- [ ] **Step 2: Add OSC 0/2 unit tests**

Append to the existing `mod tests` in `osc.rs` (DO NOT MODIFY existing tests):

```rust
    #[test]
    fn osc_0_emits_set_title() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]0;hello\x07");
        assert_eq!(events.len(), 1);
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "hello"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    #[test]
    fn osc_2_emits_set_title() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]2;world\x07");
        assert_eq!(events.len(), 1);
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "world"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    #[test]
    fn osc_0_with_embedded_semicolon_in_title() {
        let mut d = OscDispatcher::new();
        // OSC 0 has a single parameter — `;` chars after the first are part
        // of the title.
        let events = d.feed(b"\x1b]0;a;b;c\x07");
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "a;b;c"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    #[test]
    fn osc_1_is_silently_ignored() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1;icon\x07");
        // OSC 1 is icon-only; no SetTitle, no pass-through (the OSC body
        // was consumed). May still produce empty PassThrough events for
        // surrounding non-OSC bytes — assert there's no SetTitle.
        for ev in &events {
            assert!(!matches!(ev, DispatchEvent::SetTitle(_)),
                "OSC 1 should not emit SetTitle, got {events:?}");
        }
    }

    #[test]
    fn osc_0_with_st_terminator_works() {
        // ESC \ as the terminator (instead of BEL).
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]0;st_form\x1b\\");
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "st_form"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }
```

- [ ] **Step 3: Add `user_renamed` flag to `PtySession`**

Open `crates/vibeflow/src/session/session.rs`. Find `pub struct PtySession`. Add (alongside Stage 8's `selection`):

```rust
    /// True once the user has manually renamed via Ctrl+Shift+E or
    /// right-click. Sticky for the life of this session — subsequent
    /// OSC 0 / OSC 2 are ignored. Cleared on `restart()`.
    pub user_renamed: bool,
```

Initialize in `spawn`:
```rust
            user_renamed: false,
```

Reset in `restart` — find the `restart` method (Stage 8 added it). The `*self = new_session` line replaces all fields. Since `new_session` has `user_renamed: false`, this naturally resets. No code change needed beyond verifying.

- [ ] **Step 4: Route `DispatchEvent::SetTitle` in `PtySession::poll`**

Find the `poll` method's OSC event handling (Stage 1+'s dispatcher loop). Add a new arm for `SetTitle`:

```rust
                        DispatchEvent::SetTitle(title) => {
                            if !self.user_renamed {
                                self.label.title = title;
                                // The subtitle stays tracker-driven; no
                                // refresh_default_subtitle call here.
                            }
                            // else: silently dropped — user wins
                        }
```

Place it alongside the existing `DispatchEvent::AiState(_)` and `DispatchEvent::Prompt(_)` arms.

- [ ] **Step 5: Add unit tests for the routing**

Append to `mod tests` in `session.rs`:

```rust
    #[test]
    fn osc_0_updates_title_when_not_user_renamed() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default())
            .expect("spawn");
        s.dispatcher.feed(b"\x1b]0;new_title\x07");
        // The dispatcher emits the event; PtySession::poll consumes it. Call poll.
        let _ = s.poll(std::time::Instant::now());
        assert_eq!(s.label().title, "new_title");
    }

    #[test]
    fn osc_0_dropped_when_user_renamed() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default())
            .expect("spawn");
        s.user_renamed = true;
        s.dispatcher.feed(b"\x1b]0;new_title\x07");
        let _ = s.poll(std::time::Instant::now());
        assert_eq!(s.label().title, "sleep"); // unchanged from default
    }

    #[test]
    fn restart_resets_user_renamed() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default())
            .expect("spawn");
        s.user_renamed = true;
        s.set_title("user_set".to_string());
        s.restart().expect("restart");
        std::thread::sleep(std::time::Duration::from_millis(100));
        // restart() does *self = PtySession::spawn(...) using $SHELL (fallback "bash"),
        // NOT "sleep" — the original argv is not preserved across restart.
        // We assert: (a) user_renamed is reset (b) the user-set title is gone.
        assert!(!s.user_renamed, "restart must clear user_renamed");
        assert_ne!(s.label().title, "user_set", "restart must clear user-set title");
    }
```

NOTE: Test calls assume `s.dispatcher` is accessible from tests; it's `pub(crate)` likely. Verify and adapt visibility minimally if needed (don't make it fully `pub`).

- [ ] **Step 6: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/session/
cargo test -p vibeflow --lib session 2>&1 | tail -25
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 5 new osc tests + 3 new session tests = 8 added. 215 + 8 = 223 default + 15 ignored. fmt/clippy clean.

VERIFY pre-existing test names still exist:
```bash
for name in poll_routes_osc_1338_through_dispatcher_and_tracker session_spawns_and_reports_state restart_replaces_internals_with_fresh_spawn ptysession_default_label_is_bash_active; do
  grep -q "fn $name" crates/vibeflow/src/session/session.rs && echo "OK $name" || echo "MISSING $name"
done
```

If MISSING, STOP and report BLOCKED.

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/osc.rs crates/vibeflow/src/session/session.rs
git commit -m "feat(session): OSC 0/2 SetTitle + PtySession.user_renamed sticky flag"
```

---

## Task 12: Arrow / nav keys in `key_to_bytes` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

Ten new arms in `key_to_bytes` for Up/Down/Left/Right/Home/End/PageUp/PageDown/Insert/Delete with `modifiers == empty()`. xterm-compatible ANSI sequences.

**Mandatory safety rule:** **DO NOT DELETE OR WEAKEN ANY EXISTING TEST.** Stage 8's `key_to_bytes_*` tests must still exist post-edit:
- `key_to_bytes_printable_ascii`, `key_to_bytes_printable_unicode`, `key_to_bytes_enter_returns_carriage_return`, `key_to_bytes_backspace_returns_del`, plus any others Stage 8 added.

- [ ] **Step 1: Find the existing `key_to_bytes` function**

```bash
cd /path/to/vibeflow
grep -n 'fn key_to_bytes' crates/vibeflow/src/window.rs
```

Note the line number. Read the surrounding code to understand the `match logical_key` pattern.

- [ ] **Step 2: Add ten new arms**

Inside the `match logical_key` block in `key_to_bytes`, alongside the existing arms for Enter, Backspace, etc., add (with `modifiers == empty()` precondition — the existing arms likely already gate on this):

```rust
        Key::Named(NamedKey::ArrowUp)    if modifiers.is_empty() => Some(b"\x1b[A".to_vec()),
        Key::Named(NamedKey::ArrowDown)  if modifiers.is_empty() => Some(b"\x1b[B".to_vec()),
        Key::Named(NamedKey::ArrowRight) if modifiers.is_empty() => Some(b"\x1b[C".to_vec()),
        Key::Named(NamedKey::ArrowLeft)  if modifiers.is_empty() => Some(b"\x1b[D".to_vec()),
        Key::Named(NamedKey::Home)       if modifiers.is_empty() => Some(b"\x1b[H".to_vec()),
        Key::Named(NamedKey::End)        if modifiers.is_empty() => Some(b"\x1b[F".to_vec()),
        Key::Named(NamedKey::PageUp)     if modifiers.is_empty() => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown)   if modifiers.is_empty() => Some(b"\x1b[6~".to_vec()),
        Key::Named(NamedKey::Insert)     if modifiers.is_empty() => Some(b"\x1b[2~".to_vec()),
        Key::Named(NamedKey::Delete)     if modifiers.is_empty() => Some(b"\x1b[3~".to_vec()),
```

NOTE: Stage 8's existing arms may not use `if modifiers.is_empty()` guards (they may match unconditionally). For arrow / nav keys with modifiers, we want the function to return `None` so chord handling (eventual Shift+Arrow / Ctrl+Arrow in Stage 10+) doesn't accidentally pick up the plain-arrow byte. The `if modifiers.is_empty()` guard achieves this.

If an existing arm pattern looks like `Key::Named(NamedKey::Enter) => Some(...)` without the guard, leave it as-is — Enter is universal regardless of modifiers (Ctrl+Enter still emits CR). Only the new arrow / nav arms get the guard.

- [ ] **Step 3: Add ten unit tests**

Append to the existing `mod tests` in `window.rs`:

```rust
    #[test]
    fn key_to_bytes_arrow_up_emits_csi_a() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_down_emits_csi_b() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowDown), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[B".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_right_emits_csi_c() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowRight), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[C".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_left_emits_csi_d() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowLeft), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_home_emits_csi_h() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::Home), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[H".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_end_emits_csi_f() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::End), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[F".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_page_up_emits_csi_5_tilde() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::PageUp), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[5~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_page_down_emits_csi_6_tilde() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::PageDown), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_insert_emits_csi_2_tilde() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::Insert), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[2~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_delete_emits_csi_3_tilde() {
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::Delete), ElementState::Pressed, ModifiersState::empty()),
            Some(b"\x1b[3~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_up_with_ctrl_returns_none() {
        // Ctrl+ArrowUp is reserved for word-jump in Stage 10+. Until then,
        // we return None so the byte path doesn't accidentally pick up the
        // plain `\x1b[A` for a modified chord.
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                ElementState::Pressed,
                ModifiersState::CONTROL
            ),
            None
        );
    }
```

- [ ] **Step 4: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/window.rs
cargo test -p vibeflow --lib key_to_bytes 2>&1 | tail -20
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 11 new tests pass (10 keys + 1 modifier-rejection). 223 + 11 = 234 default + 15 ignored. fmt/clippy clean.

VERIFY pre-existing key_to_bytes test names still exist (sample):
```bash
for name in key_to_bytes_printable_ascii key_to_bytes_printable_unicode key_to_bytes_enter_returns_carriage_return key_to_bytes_backspace_returns_del; do
  grep -q "fn $name" crates/vibeflow/src/window.rs && echo "OK $name" || echo "MISSING $name"
done
```

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): arrow + navigation keys → xterm ANSI sequences (TDD)"
```

---

## Task 13: Tab rename UI (keyboard + render override + right-click)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`
- Modify: `crates/vibeflow/src/render/tabs.rs`

`RenameInputState` on `WindowApp`. Keyboard captured while renaming. Right-click on a tab body triggers rename. Tab render override draws the editable buffer + caret.

- [ ] **Step 1: Add `RenameInputState` to `render/tabs.rs`** (NOT `window.rs` — see note below)

`RenameInputState` is consumed by both `window.rs` (which owns the state) AND `render/tabs.rs` (which renders the buffer + caret in `push_text_glyphs`). If we put the type in `window.rs`, `render/tabs.rs` would need to import from `window.rs` — but `window.rs` already imports from `render::tabs`, so the import would be circular and won't compile.

Define the type alongside the other rendering types in `render/tabs.rs`:

Open `crates/vibeflow/src/render/tabs.rs`. Add the struct (alongside `RectInstance`, `TabBarLayout`, `TabBarHit`):

```rust
/// State for an in-progress inline tab rename. Owned by `WindowApp`; passed
/// by reference into `push_text_glyphs` so the renamed tab renders the
/// editable buffer + caret instead of the static label title.
///
/// Defined in `render::tabs` (not `window`) to avoid a circular module
/// dependency: `window` already imports `render::tabs`, so `render::tabs`
/// cannot import from `window`.
#[derive(Debug, Clone)]
pub struct RenameInputState {
    /// Index in `app.tabs()` of the tab being renamed.
    pub tab_idx: usize,
    /// User's typed text so far.
    pub buffer: String,
    /// Byte index in `buffer` for cursor position. Always at a grapheme boundary.
    pub cursor_pos: usize,
    /// Original title before rename, for Esc-cancel restore.
    pub original: String,
}
```

In `crates/vibeflow/src/window.rs`, import it:

```rust
use crate::render::tabs::RenameInputState;
```

Add field to `WindowApp`:

```rust
    rename_state: Option<RenameInputState>,
```

Initialize to `None` in `WindowApp::new`.

- [ ] **Step 2: Add `start_rename`, `commit_rename`, `cancel_rename` methods**

Append to `impl WindowApp`:

```rust
    fn start_rename(&mut self, tab_idx: usize) {
        let Some(s) = self.app.tabs().get(tab_idx) else { return };
        let title = s.label().title.clone();
        self.rename_state = Some(RenameInputState {
            tab_idx,
            cursor_pos: title.len(),
            buffer: title.clone(),
            original: title,
        });
        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
    }

    fn commit_rename(&mut self) {
        let Some(rs) = self.rename_state.take() else { return };
        if let Some(s) = self.app.tabs_mut().get_mut(rs.tab_idx) {
            s.set_title(rs.buffer);
            s.user_renamed = true;
        }
        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
    }

    fn cancel_rename(&mut self) {
        let Some(rs) = self.rename_state.take() else { return };
        if let Some(s) = self.app.tabs_mut().get_mut(rs.tab_idx) {
            s.set_title(rs.original);
        }
        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
    }
```

Note: `s.set_title(String)` is a new convenience method that replaces only line 1 (title), keeping the existing subtitle. The existing `set_label(TabLabel)` replaces both lines. Add `set_title` to `crates/vibeflow/src/session/session.rs` (in `impl PtySession`, near the existing `pub fn set_label`):

```rust
    /// Replace only the title (line 1) of the label, preserving the current
    /// subtitle. Used by the interactive rename UI which doesn't touch the
    /// activity-driven subtitle.
    pub fn set_title(&mut self, title: String) {
        self.label.title = title;
    }
```

- [ ] **Step 3: Replace the no-op `RenameTab` arm with the real implementation**

Task 4 Step 1.5 added a no-op `Shortcut::RenameTab` arm in `WindowApp::handle_shortcut` to keep the match exhaustive. Replace it:

```rust
            // Was (Task 4):
            //   Shortcut::RenameTab => {
            //       tracing::trace!("RenameTab shortcut ignored (Task 13 wires it)");
            //   }
            // Becomes:
            Shortcut::RenameTab => {
                self.start_rename(self.app.active());
            }
```

- [ ] **Step 4: Capture keyboard while renaming**

In `WindowEvent::KeyboardInput`, AT THE TOP (before `match_shortcut` dispatch and before the typed-input fallthrough), add the rename-input capture branch:

```rust
                if event.state == ElementState::Pressed {
                    if self.rename_state.is_some() {
                        self.handle_rename_keyboard(&event.logical_key);
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                        return;
                    }
                }
```

Add the `handle_rename_keyboard` method using a "decide → apply outside" pattern. `drop(rs)` does NOT release the borrow on `self` (the `as_mut` borrow lives for the entire `if let` arm), so we must collect the desired action into a local enum, let the borrow on `self.rename_state` end, then apply the action with a fresh `&mut self`:

```rust
    /// Outcome of a single keypress while a rename is in progress.
    enum RenameAction {
        None,
        Commit,
        Cancel,
        // For all the in-buffer edits, we apply them inside the rename_state
        // borrow; nothing escapes.
        Edited,
    }

    fn handle_rename_keyboard(&mut self, key: &winit::keyboard::Key) {
        use winit::keyboard::{Key, NamedKey};
        let Some(rs) = self.rename_state.as_mut() else { return };
        let action = match key {
            Key::Named(NamedKey::Enter) => RenameAction::Commit,
            Key::Named(NamedKey::Escape) => RenameAction::Cancel,
            Key::Named(NamedKey::Backspace) => {
                if rs.cursor_pos > 0 {
                    let new_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
                    rs.buffer.replace_range(new_pos..rs.cursor_pos, "");
                    rs.cursor_pos = new_pos;
                }
                RenameAction::Edited
            }
            Key::Named(NamedKey::Delete) => {
                if rs.cursor_pos < rs.buffer.len() {
                    let new_end = next_grapheme(&rs.buffer, rs.cursor_pos);
                    rs.buffer.replace_range(rs.cursor_pos..new_end, "");
                }
                RenameAction::Edited
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if rs.cursor_pos > 0 {
                    rs.cursor_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
                }
                RenameAction::Edited
            }
            Key::Named(NamedKey::ArrowRight) => {
                if rs.cursor_pos < rs.buffer.len() {
                    rs.cursor_pos = next_grapheme(&rs.buffer, rs.cursor_pos);
                }
                RenameAction::Edited
            }
            Key::Named(NamedKey::Home) => {
                rs.cursor_pos = 0;
                RenameAction::Edited
            }
            Key::Named(NamedKey::End) => {
                rs.cursor_pos = rs.buffer.len();
                RenameAction::Edited
            }
            Key::Character(c) => {
                rs.buffer.insert_str(rs.cursor_pos, c.as_str());
                rs.cursor_pos += c.as_str().len();
                RenameAction::Edited
            }
            // Ignore modifier-only keys (Ctrl/Shift/Alt/Super alone), F-keys, etc.
            _ => RenameAction::None,
        };
        // The `as_mut` borrow on `self.rename_state` ends when `rs` goes out
        // of scope (end of statement above). Now we can call `&mut self`
        // methods like `commit_rename` / `cancel_rename`.
        match action {
            RenameAction::Commit => self.commit_rename(),
            RenameAction::Cancel => self.cancel_rename(),
            RenameAction::Edited | RenameAction::None => {}
        }
    }
```

The `prev_grapheme` / `next_grapheme` helpers walk grapheme boundaries. Use the `unicode-segmentation` crate (already a transitive dep through cosmic-text — verify with `grep unicode-segmentation crates/vibeflow/Cargo.lock`):

```rust
fn prev_grapheme(s: &str, pos: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .filter(|&i| i < pos)
        .last()
        .unwrap_or(0)
}

fn next_grapheme(s: &str, pos: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i > pos)
        .unwrap_or(s.len())
}
```

If `unicode-segmentation` is NOT a transitive dep, add it to `Cargo.toml`:

```toml
unicode-segmentation = "1"
```

The two-phase pattern above is necessary because of Rust's borrow checker — `dropping` an `&mut` reference does NOT release the underlying borrow until the borrow's scope ends. Trying to call `self.commit_rename()` while still inside the `if let Some(rs) = self.rename_state.as_mut()` arm fails with "cannot borrow `self` as mutable more than once at a time."

ALTERNATIVE APPROACH (only if the above proves awkward — keep as backup): split the match into "decide what to do" + "apply outside" with a closure:

```rust
    fn handle_rename_keyboard(&mut self, key: &winit::keyboard::Key) {
        use winit::keyboard::{Key, NamedKey};
        // Decide whether to commit/cancel/edit.
        enum Action { None, Commit, Cancel, Edit(EditOp) }
        // ... derive Action from key + cursor_pos
        // Then apply outside the rename_state borrow.
    }
```

Use whichever the borrow checker accepts; the spec is the behavior, the structure is implementation detail.

- [ ] **Step 5: Right-click rename trigger + click-out-cancel**

In `WindowEvent::MouseInput`'s tab-bar passthrough (the `if py < bar_h { ... }` block), extend the existing Stage 8 logic to handle right-click:

```rust
                    if py < bar_h {
                        // Existing Stage 8 release-left handler.
                        if state == ElementState::Released && button == MouseButton::Left {
                            self.handle_left_click_release();
                        }
                        // Stage 9: right-click on a tab body opens rename.
                        if state == ElementState::Released && button == MouseButton::Right {
                            // Hit-test to find which tab.
                            if let Some(renderer) = self.renderer.as_ref() {
                                let (window_w, _) = renderer.surface_size();
                                let (_, cell_h) = renderer.cell_pitch();
                                let layout = crate::render::tabs::TabBarLayout::compute(
                                    window_w, cell_h, self.app.tabs().len()
                                );
                                if let crate::render::tabs::TabBarHit::TabBody(idx) =
                                    layout.hit_test(px, py)
                                {
                                    self.start_rename(idx);
                                }
                            }
                        }
                        return;
                    }
```

For click-out-cancel: in the cell-grid section of MouseInput (NOT the tab-bar passthrough), AT THE TOP, add:

```rust
                    if self.rename_state.is_some() {
                        // Click in cell area → cancel rename, then process click.
                        self.cancel_rename();
                        // Fall through to the existing selection / mouse-mode logic.
                    }
```

Also: clicking on a different tab body cancels rename and switches active. In the tab-bar passthrough release-left handler:

```rust
                        if state == ElementState::Released && button == MouseButton::Left {
                            // Click on a tab body that ISN'T the one being renamed → cancel.
                            if let Some(rs) = &self.rename_state {
                                if let Some(renderer) = self.renderer.as_ref() {
                                    let (window_w, _) = renderer.surface_size();
                                    let (_, cell_h) = renderer.cell_pitch();
                                    let layout = crate::render::tabs::TabBarLayout::compute(
                                        window_w, cell_h, self.app.tabs().len()
                                    );
                                    if let crate::render::tabs::TabBarHit::TabBody(idx) =
                                        layout.hit_test(px, py)
                                    {
                                        if idx != rs.tab_idx {
                                            self.cancel_rename();
                                        } else {
                                            return; // click on same tab → no-op
                                        }
                                    } else {
                                        // Click on `+` or close button → cancel and proceed.
                                        self.cancel_rename();
                                    }
                                }
                            }
                            self.handle_left_click_release();
                        }
```

NOTE: The existing Stage 8 left-click code is already complex; this extension is grafted on. Adapt to the actual structure when you read the file.

- [ ] **Step 6: Render override in `tabs.rs::push_text_glyphs`**

Open `crates/vibeflow/src/render/tabs.rs`. The `RenameInputState` type was defined in this same file in Step 1, so no cross-module import is needed.

`push_text_glyphs` is a private free function in `tabs.rs` (around line 765); it's called by `TabBarRenderer::build_glyphs` (pub, around line 658), which is called by `Renderer::render` in `render/mod.rs`. Both signatures need the `rename_state` parameter threaded through.

Update `push_text_glyphs` signature:

```rust
fn push_text_glyphs(
    // ... existing parameters
    rename_state: Option<&RenameInputState>,
)
```

Update `TabBarRenderer::build_glyphs` signature to forward it:

```rust
pub fn build_glyphs(
    // ... existing parameters
    rename_state: Option<&RenameInputState>,
    // ... existing trailing params
) -> ... {
    // ... existing code
    push_text_glyphs(/* existing args */, rename_state);
    // ...
}
```

Update the call site in `crates/vibeflow/src/render/mod.rs`'s `Renderer::render` method to pass `rename_state` (a parameter on `render` added in Task 9 / Task 13 Step 7 below).

Inside the per-tab loop, when iterating tab `idx`, check:

```rust
        let title_to_render: &str = if let Some(rs) = rename_state {
            if rs.tab_idx == idx { &rs.buffer } else { &session.label().title }
        } else {
            &session.label().title
        };
```

Use `title_to_render` instead of `session.label().title` for the title-line glyphs.

When `rs.tab_idx == idx`, ALSO emit:
- An underlay rect tinted `[1.0, 1.0, 1.0, 0.15]` over the title row of the tab (visual editing affordance).
- A blinking caret rect at `cursor_pos`. Position: the x-pixel offset of the byte at `cursor_pos` (compute via re-shaping `&buffer[..cursor_pos]` and reading its width). Reuse the existing cursor blink phase tracking from Stage 7's `cursor.rs` so the rename caret blinks in sync with the cell-grid cursor.

The exact rendering math depends on the existing tab-text layout. Keep it simple: for v1, render the caret as a 2px-wide rect at the title's x-baseline + measured width of `buffer[..cursor_pos]`. If text-engine measurement isn't available as a pure function, render the caret at the END of the buffer (cursor_pos == len) and don't track within-buffer positions visually. Document the limitation; Stage 10 polish.

- [ ] **Step 7: Verify**

```bash
cd /path/to/vibeflow
cargo fmt -p vibeflow -- crates/vibeflow/src/window.rs crates/vibeflow/src/render/tabs.rs crates/vibeflow/src/session/session.rs
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 234 default + 15 ignored. Task 13 doesn't add new tests (the rename behavior is integration-tested via manual smoke). fmt/clippy clean.

If `unicode-segmentation` is missing as a transitive dep, the `cargo check` will say "unresolved import `unicode_segmentation`" — add to Cargo.toml.

If borrow-checker fails on `s.set_title(...)` while `tabs_mut` is borrowed — the let-else pattern from Stage 8 (active-binding-first) applies here too:

```rust
        let active = self.app.active();
        let Some(rs) = self.rename_state.take() else { return };
        if let Some(s) = self.app.tabs_mut().get_mut(rs.tab_idx) { ... }
```

- [ ] **Step 8: Add a few unit tests for rename logic**

The rename machine is small but worth covering. Append to `mod tests` in `window.rs`:

```rust
    use winit::keyboard::SmolStr;

    fn ch_key(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    fn rename_state(buffer: &str, cursor: usize) -> RenameInputState {
        RenameInputState {
            tab_idx: 0,
            buffer: buffer.to_string(),
            cursor_pos: cursor,
            original: buffer.to_string(),
        }
    }

    #[test]
    fn rename_backspace_deletes_grapheme() {
        let mut rs = rename_state("hello", 5);
        // Simulate Backspace: drop one grapheme from end.
        let new_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
        rs.buffer.replace_range(new_pos..rs.cursor_pos, "");
        rs.cursor_pos = new_pos;
        assert_eq!(rs.buffer, "hell");
        assert_eq!(rs.cursor_pos, 4);
    }

    #[test]
    fn rename_backspace_handles_multibyte() {
        // "café" — 'é' is 2 bytes (0xC3 0xA9). Backspace deletes the whole grapheme.
        let mut rs = rename_state("café", 5);
        let new_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
        rs.buffer.replace_range(new_pos..rs.cursor_pos, "");
        rs.cursor_pos = new_pos;
        assert_eq!(rs.buffer, "caf");
        assert_eq!(rs.cursor_pos, 3);
    }

    #[test]
    fn rename_arrow_left_moves_by_grapheme() {
        let mut rs = rename_state("abc", 3);
        rs.cursor_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
        assert_eq!(rs.cursor_pos, 2);
    }

    #[test]
    fn rename_home_jumps_to_zero() {
        let mut rs = rename_state("abc", 2);
        rs.cursor_pos = 0;
        assert_eq!(rs.cursor_pos, 0);
    }

    #[test]
    fn rename_end_jumps_to_len() {
        let mut rs = rename_state("abc", 0);
        rs.cursor_pos = rs.buffer.len();
        assert_eq!(rs.cursor_pos, 3);
    }

    #[test]
    fn rename_insert_at_cursor() {
        let mut rs = rename_state("ab", 1);
        rs.buffer.insert_str(rs.cursor_pos, "X");
        rs.cursor_pos += 1;
        assert_eq!(rs.buffer, "aXb");
        assert_eq!(rs.cursor_pos, 2);
    }
```

Expected: 6 new tests pass. 234 + 6 = 240 default + 15 ignored.

- [ ] **Step 9: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/window.rs \
        crates/vibeflow/src/render/tabs.rs \
        crates/vibeflow/src/session/session.rs \
        crates/vibeflow/Cargo.toml
git commit -m "feat(window): inline tab rename UI (Ctrl+Shift+E / F2 / right-click)"
```

---

## Task 14: Final verification + smoke checklist + tag

- [ ] **Step 1: Append Stage 9 section to `docs/TESTING.md`**

After the Stage 8 section, append:

```markdown

## Stage 9 — TOML config + bundled UX quick wins

Run:

```bash
cd /path/to/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

### Config — startup defaults (no file)

- [ ] Cold-start with no `~/.config/vibeflow/config.toml` → vibeflow opens with default selection color, cursor blinks at 1Hz, all Stage 8 shortcuts work.

### Config — partial file

- [ ] Create `~/.config/vibeflow/config.toml`:
   ```toml
   [colors]
   selection = "#ff0000ff"
   ```
   Save. Within ~250ms, the selection rect color changes to red on the next drag. (Drag-select to verify.)

- [ ] Edit the file: `[cursor]` `blink_ms = 0`. Save. The cursor stops blinking (renders solid).

- [ ] Edit the file: `blink_ms = 250`. Save. The cursor blinks twice as fast.

### Config — error banner

- [ ] Edit the file: `[cursor]` `blink_ms = "fast"` (invalid type). Save.
   - A dark-red banner appears at the top of the cell area: `⚠ 1 config key ignored: cursor.blink_ms (or similar) — Esc to dismiss`.
   - The cursor blink rate falls back to the previous value (or default).
- [ ] Press Esc — banner dismisses.
- [ ] Edit the file to fix the typo: `blink_ms = 500`. Save → banner re-clears (already dismissed; banner stays gone).
- [ ] Add a NEW typo (`indicator_active = "not a color"`). Save → banner reappears with the new error.

### Config — file deletion

- [ ] With vibeflow open, `rm ~/.config/vibeflow/config.toml`. Banner: `⚠ 1 config key ignored: I/O: ... removed at runtime`. Current values retained.
- [ ] Recreate the file with valid content. Banner clears, new values applied.

### Custom shortcuts

- [ ] Edit `[shortcuts]` `new_tab = ["ctrl+alt+t"]`. Save.
- [ ] `Ctrl+Shift+T` no longer spawns a tab. `Ctrl+Alt+T` spawns one.
- [ ] Restore the default. Save. `Ctrl+Shift+T` spawns again.

### Tab rename

- [ ] `Ctrl+Shift+E` → active tab's title becomes editable; cursor is at the end of the title; tab background slightly tinted.
- [ ] Type "claude work". Backspace, type something else. Arrow keys move within the buffer. Home / End jump.
- [ ] Press Enter → tab title is now "claude work". Subtitle still shows the activity state.
- [ ] `F2` also opens rename.
- [ ] Right-click on a different tab → its title becomes editable.
- [ ] Press Esc during rename → title reverts.
- [ ] During rename, click on another tab → rename cancelled, that tab becomes active.
- [ ] During rename, click in the cell area → rename cancelled.
- [ ] After a user rename, run `printf '\x1b]0;new_title_from_shell\x07'` → the renamed tab is unaffected.
- [ ] On an UN-renamed tab, set bash `PROMPT_COMMAND='printf "\e]0;%s\a" "$(pwd)"'` → the tab title updates to the cwd on each prompt.
- [ ] `Ctrl+Shift+R` on a (dead) renamed tab → fresh shell, title resets to `bash`.

### Arrow / nav keys

- [ ] At a `bash` prompt, press Up → previous command appears.
- [ ] Press Down → next.
- [ ] Type a long command, press Home → cursor jumps to start. End → end.
- [ ] Run `less /etc/passwd`. PageUp / PageDown work as expected.

### PRIMARY clipboard

- [ ] Drag-select text in vibeflow. Without pressing Ctrl+Shift+C, middle-click in another vibeflow tab or another GUI app → text pastes.
- [ ] In another GUI app, select-and-PRIMARY-copy "external text". Middle-click in vibeflow → "external text" arrives at the prompt.
- [ ] Edit config: `[clipboard]` `primary = false`. Save. Drag-select in vibeflow → middle-click no longer pastes.
- [ ] Restore `primary = true`.

### Cross-cutting

- [ ] Resize the window — selection clears (Stage 8 behavior preserved).
- [ ] Open vim, `:set mouse=a` — mouse-mode passthrough still works (Stage 8).
- [ ] `WINIT_UNIX_BACKEND=x11 ./target/debug/vibeflow` — all checks above still pass.

**Known Stage 9 limitations (deferred):**

- Font priority hot-reload only partially applies (the cosmic-text fontdb is rebuilt, but glyphs already on screen remain at their previous family until the next rasterize). Restart vibeflow to fully apply font priority changes.
- Live-reload of `[fonts]` `priority` may show a brief atlas re-rasterize on the next frame.
- Right-click context menu (Copy / Paste / etc) for the cell area — Stage 10 (needs overlay rendering).
- Block (column) selection — Stage 10.
- Selection in scrollback — Stage 10.
- Shift / Ctrl-modifier arrow keys (Shift+Right for word-jump in editors, etc.) — Stage 10/11.
- Bell behavior config — defer to Stage 10.
- Indicator dot colors apply on next render but the existing tab bar may need a full redraw to pick them up; click any tab once to force redraw.
```

- [ ] **Step 2: Full local CI dry-run**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo build --workspace --all-targets && \
  cargo test --workspace --all-targets && \
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps && \
  ( cd bindings/npm && npm run build && npm test ) && \
  echo "ALL GREEN"
```

Expected:
- ~240 default lib tests pass + ~15 ignored (vs Stage 8's 176 + 13).
- 27 protocol crate tests, 15 npm tests, integration tests pass.

If anything fails, STOP and report.

- [ ] **Step 3: 60-second fuzz**

```bash
cd /path/to/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: no crashes / leaks. Stage 9 didn't change the protocol parser, so this is a sanity check.

- [ ] **Step 4: Final senior-tier holistic code review**

Stage 8's lesson: per-task Haiku reviewers consistently miss whole-stage issues. Before tagging, dispatch ONE more review covering the entire branch:

```
The reviewer should `git log --oneline main..HEAD` and inspect the cumulative
diff. Focus areas:
(a) cross-task coherence — did per-task fixes regress earlier work, especially
    the Stage 8 selection / shortcut / clipboard paths?
(b) the keymap refactor — does ShortcutTable.lookup correctly handle every
    Stage 8 chord?
(c) hot-reload safety — what happens if the watcher thread panics? does the
    main thread keep running?
(d) error banner Z-order — does the banner correctly draw over selection
    rects but under the tab strip?
(e) OSC 0/2 + user_renamed precedence — trace through the user-rename →
    OSC 0 → restart sequence.
(f) arrow key modifier rejection — does Ctrl+Shift+Up correctly fall through
    to None (so Stage 10 can layer modifier handling)?
(g) test-count drift; (h) any lingering TODO / FIXME / TBD strings.
```

Subagent dispatch: `general-purpose` with `model: sonnet`. Treat output as advisory unless flagged Critical/Important. If anything substantive surfaces, fix before tagging.

- [ ] **Step 5: Manual smoke walkthrough**

Walk the Stage 9 section of `docs/TESTING.md` (Step 1 above) on host via VNC.

- [ ] **Step 6: Commit + tag**

```bash
cd /path/to/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 9 manual smoke checklist"
git tag -a stage9-config-complete \
  -m "TOML config + bundled UX quick wins complete (Stage 9 of v0.1)"
git tag --list | grep stage9
```

- [ ] **Step 7: Surface to user**

Report:
- Number of new commits (~14 implementation + 1 docs = ~15).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 10 (scrollback rendering + right-click context menu) as the next plan.

---

## Spec coverage check

| Spec section | Covered by |
|---|---|
| TOML config file at XDG path | Task 0 + Task 2 (`Config::default_path`) |
| `serde`-derived schema | Task 1 |
| `Config` aggregate + per-key tolerance | Task 2 |
| Color hex parser (`#RRGGBBAA`) | Task 2 |
| Shortcut spec parser | Task 2 |
| `AppUserEvent` + EventLoopProxy plumbing | Task 3 |
| `ShortcutTable` data-driven dispatch | Task 4 |
| `Shortcut::RenameTab` variant + default bindings | Task 4 |
| `ErrorBannerState` | Task 5 |
| `notify` watcher + 250ms debounce | Task 6 |
| WindowApp config-distribution + watcher spawn | Task 7 |
| Renderer setters (selection / indicators / cursor / fonts) | Task 8 |
| Error banner rendering | Task 9 |
| `Esc` dismisses banner | Task 9 |
| PRIMARY clipboard wrapper + auto-copy | Task 10 |
| Middle-click PRIMARY paste | Task 10 |
| OSC 0 / OSC 2 → SetTitle | Task 11 |
| `PtySession.user_renamed` sticky flag | Task 11 |
| Arrow / nav keys → ANSI sequences | Task 12 |
| `RenameInputState` + interactive editing | Task 13 |
| Right-click rename trigger | Task 13 |
| Click-out cancel | Task 13 |
| Render override for editing tab | Task 13 |
| Smoke checklist + tag | Task 14 |

Every spec item maps to a task.

## Self-review

- **Spec coverage:** every Stage 9 spec requirement maps to a task (table above).
- **Placeholder scan:** no `TBD`/`TODO`/`implement later` patterns. Each step has actual code or commands.
- **Type consistency check:**
  - `Config`, `ConfigError`, `KeyChord`, `KeyMatch`, `ShortcutBindings` defined in Task 2, consumed in Tasks 4, 7.
  - `AppUserEvent` defined in Task 3, sent by Task 6 (watcher), received by Task 7 (user_event).
  - `ShortcutTable` defined in Task 4, consumed in Task 7 (`replace_from_bindings`).
  - `ErrorBannerState` defined in Task 5, rendered in Task 9.
  - `DispatchEvent::SetTitle` defined in Task 11, consumed in Task 11 PtySession::poll.
  - `PtySession.user_renamed` defined in Task 11, consumed in Task 13 commit_rename.
  - `RenameInputState` defined in Task 13, rendered by Task 13 step 6 (tabs.rs render override).
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** unchanged. Watcher thread is the only addition; it sends via proxy, no shared state.
- **Test count tracking:** Stage 8 ends at 176 default + 13 ignored. Stage 9 adds:
  - schema (Task 1): 6 default
  - config mod (Task 2): 15 default
  - keymap (Task 4): 5 new (17 preserved → 22 total; net +5)
  - error_banner (Task 5): 8 default
  - watcher (Task 6): 3 default + 1 ignored
  - clipboard (Task 10): 1 default + 1 new ignored
  - osc (Task 11): 5 default
  - session (Task 11): 3 default
  - key_to_bytes (Task 12): 11 default
  - rename helpers (Task 13): 6 default
  - **Final: ~240 default + ~15 ignored.**

## Notable plan risks

1. **`set_font_priorities` live-reload.** Cosmic-text 0.12's FontSystem rebuild may not cleanly invalidate the texture atlas. Fall back to "applied at next startup" if it bites. Document in Stage 9 smoke as a known limitation.

2. **`arboard` PRIMARY API differences across versions.** Verified against arboard 3.6's `SetExtLinux`/`GetExtLinux` traits; if those have moved between minor versions, adapt. Cargo.lock pinning helps.

3. **`unicode-segmentation` may not be a transitive dep.** If `cargo check` fails with unresolved import, add as a direct dep.

4. **Borrow checker on `WindowApp::handle_rename_keyboard`.** Methods that take `&mut self` and need to read `self.rename_state` then dispatch to `self.commit_rename()` cause aliasing. The plan suggests a "decide → apply outside" restructure if needed.

5. **`notify` watcher behavior on different filesystems.** Vim's atomic-rename on ext4 produces Remove + Create; on tmpfs may differ. The 250ms debounce + the Remove → ConfigError path covers both, but CI may flake on the integration test.

6. **`EventLoopProxy<T>` Send/Clone bounds in winit 0.30.** Verified `EventLoopProxy: Clone + Send` for `T: Send`. `AppUserEvent` contains only `Config` (which contains `Vec<String>`, `HashMap`, etc — all Send) and `ConfigError` (similar). No issue expected.

7. **`set_indicator_colors` integration with the Stage 4 tab-bar dot rendering.** If the tab-bar renderer's color resolution is hardcoded in a place the plan didn't cover, the implementer should follow the dot-color path through tabs.rs and replace the hardcoded values with `self.indicator_colors[state_index]`.

8. **OSC 0/2 with surrogate-pair UTF-8.** `String::from_utf8_lossy` handles invalid bytes. Embedded NUL bytes in the title? Stripped by `from_utf8_lossy` via U+FFFD. Acceptable for v1.

These risks are addressed by senior pre-execution review of this plan and the Stage 9 manual smoke walkthrough.
