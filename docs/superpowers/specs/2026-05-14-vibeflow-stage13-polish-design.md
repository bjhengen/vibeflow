# vibeflow — Stage 13: Polish bucket (indicator + bell + themes + input + font)

**Status:** Draft, pending review
**Date:** 2026-05-14
**Author:** brainstormed with Claude

## Summary

Stage 13 is the v0.1 polish bucket — seven mostly-independent features bundled into one stage. Most are small focused changes (single-line constant bump, one or two new `key_to_bytes` arms, a new config knob). The biggest is the **theme system**: import iTerm2 `.itermcolors` files into a per-user theme registry, set a default theme via config, and override per-tab at runtime via the Stage 10 context menu.

Brian prioritized indicator visual prominence ("important to the overall project"). Stage 11's smoke walk showed the 3 px stripe + α 0.4↔1.0 pulse was too subtle to read as "needs attention." Stage 13 widens the stripe to 6 px (chosen from five mockups) — minimum-friction fix; pulse range stays the same; rest of the indicator visual treatment unchanged.

After Stage 13, v0.1 has one stage left (Stage 14 ship: README, `cargo install`, packaging, v0.1 tag). Tier 2 wrapper shims are an optional sub-stage that can land before or after ship.

## Goals & Non-Goals

### Goals (Stage 13)

1. **Indicator prominence**: bump `INDICATOR_STRIPE_WIDTH_PX` from 3 → 6. Hardcoded; no new config knob.
2. **Bell behavior config**: new `[bell]` section with `mode = "visual" | "audible" | "both" | "silent"` (default `"visual"`) and `debounce_ms = 100` (default; ignore 0x07 bytes closer together).
3. **iTerm2 theme system**: `vibeflow --import-colors <path>` CLI, theme registry at `~/.config/vibeflow/themes/`, `[colors] preset = "<name>"` for default, per-tab override via context menu.
4. **Shift / Ctrl modifier arrow keys**: standard xterm escape sequences for word/line jump.
5. **Block (column) selection**: Alt+drag yields rectangular selection; `text()` joins rows with `\n`.
6. **Shift-extend selection anchor**: shift-click extends the existing anchor instead of starting a fresh selection.
7. **Esc snap-to-bottom config knob**: `[scrollback] snap_on_esc = true` (default matches Stage 12 behavior) so users who don't want Esc to snap can opt out.
8. **Font priority live-reload**: rebuild the `cosmic_text::FontSystem`'s database when `[fonts] priority` changes, so the new order applies immediately (today it requires a restart).

### Non-Goals (Stage 13)

- Context menu submenus (theme list is flat for v0.1; submenus are v0.2 polish per the brainstorm).
- Persistent per-tab themes across vibeflow restart (no tab-identity infrastructure; per-tab override is session-scoped).
- Indicator stripe-width config knob (YAGNI — hardcoded 6 px, revisit if users actually want to tune).
- Tier 2 wrapper shims for codex / opencode (separate stage 13b or post-v0.1).
- Cross-platform audible bell (Linux-only via `paplay`; falls back to silent on systems without).
- Theme picker overlay UI (use the context menu list).

## Architecture

### Module layout

| File | Status | Responsibility |
|---|---|---|
| `crates/vibeflow/src/theme/mod.rs` | NEW | `pub struct ThemeData` with ANSI palette + special colors; root of the theme module. |
| `crates/vibeflow/src/theme/iterm2.rs` | NEW | `parse_itermcolors(plist_bytes: &[u8]) -> Result<ThemeData, ItermImportError>` — pure plist parser. |
| `crates/vibeflow/src/theme/registry.rs` | NEW | `ThemeRegistry::load(themes_dir)` scans `~/.config/vibeflow/themes/*.toml`; provides `get`, `names`, `reload`. |
| `crates/vibeflow/src/main.rs` | TOUCHED | Parse `--import-colors <path>` argv; if present, write theme.toml and exit before launching GUI. New `--overwrite` flag for replacing existing themes. |
| `crates/vibeflow/src/lib.rs` | TOUCHED | Add `pub mod theme;`. |
| `crates/vibeflow/src/render/tabs.rs` | TOUCHED | `INDICATOR_STRIPE_WIDTH_PX: 3 → 6`. |
| `crates/vibeflow/src/render/bell.rs` | TOUCHED (light) | No code change; comments updated to reference new modes. |
| `crates/vibeflow/src/render/selection.rs` | TOUCHED | `SelectionMode::Block` variant; `cells_in_range_block`; `cells()`/`text()` dispatch on mode; `mouse_down` accepts `alt: bool`; shift-extend anchor refinement. |
| `crates/vibeflow/src/render/text_engine.rs` | TOUCHED | `set_font_priorities` rebuilds `FontSystem::db_mut()` immediately. |
| `crates/vibeflow/src/render/context_menu.rs` | TOUCHED | `MenuAction::SetTheme(String)` variant; tab-menu builder accepts theme registry to enumerate available themes. |
| `crates/vibeflow/src/session/session.rs` | TOUCHED | `PtySession.theme: Option<String>` field; `set_theme(name, registry)` method; `restart()` preserves; spawn signature gains theme parameter (or App reads default at spawn time). |
| `crates/vibeflow/src/app.rs` | TOUCHED | `default_theme: Option<String>` field + `set_default_theme` setter; `new_tab` / `restart_active` propagation. |
| `crates/vibeflow/src/window.rs` | TOUCHED | New cache fields (`bell_mode`, `bell_debounce`, `last_bell_at`, `snap_on_esc`); `WindowApp.theme_registry: ThemeRegistry`; `key_to_bytes` extended for arrow-key chords; `handle_session_event` for `Bell` dispatches on mode; Stage 12 snap-to-bottom hook gates on `snap_on_esc` for Esc; mouse handler passes `alt` to selection.mouse_down; `apply_config` wires `[bell]` + `[colors] preset` + `[scrollback] snap_on_esc`; menu action dispatch for `SetTheme`. |
| `crates/vibeflow/src/config/schema.rs` | TOUCHED | New `BellSection`; `ColorsSection.preset: Option<String>`; `ScrollbackSection.snap_on_esc: Option<bool>`. |
| `crates/vibeflow/src/config/mod.rs` | TOUCHED | New resolved `Bell` struct + `Colors.preset` field + `Scrollback.snap_on_esc` field + defaults + apply steps. |
| `crates/vibeflow/Cargo.toml` | TOUCHED | Add `plist = "1"` dependency. |
| `crates/vibeflow/tests/themes.rs` | NEW | Integration tests for iTerm2 parsing + registry scanning + apply-via-config. |

### Data flow — theme application

```
.itermcolors file               (user's iTerm2 export)
        │
        ▼  vibeflow --import-colors
   theme::iterm2::parse_itermcolors
        │
        ▼  serde TOML emit
   ~/.config/vibeflow/themes/<name>.toml
        │
        ▼  vibeflow startup: ThemeRegistry::load
   ThemeRegistry (HashMap<String, ThemeData>)
        │
        ├──→ [colors] preset = "X" in config.toml
        │     │
        │     ▼  WindowApp::apply_config
        │   App::default_theme = Some("X")
        │     │
        │     ▼  apply to all existing tabs + future new_tab spawns
        │   PtySession::set_theme(Some("X"), &registry)
        │     │
        │     ▼  Term::colors_mut() writes ANSI palette + special
        │   Term renders with theme colors
        │
        └──→ context menu right-click on tab → SetTheme(name) item
              │
              ▼  WindowApp::dispatch_menu_action
            PtySession::set_theme(Some(name), &registry)  on target tab only
              │
              ▼  same Term::colors_mut() flow
            Per-tab override (not persisted across vibeflow restart;
            survives tab restart via PtySession::restart())
```

### Data flow — bell

```
0x07 byte in PTY output         (BEL)
        │
        ▼  Stage 6: OscDispatcher emits SessionEvent::Bell
   PtySession::poll → returns Bell event
        │
        ▼  WindowApp::handle_session_event
   match event:
     Bell =>
       │
       ▼  Stage 13: debounce check
     if now - last_bell_at < bell_debounce: return
     last_bell_at = Some(now)
       │
       ▼  Stage 13: dispatch on mode
     match bell_mode:
       Silent => no-op
       Visual => bell_flash.trigger(now)              ← existing Stage 6 path
       Audible => spawn(paplay /usr/share/sounds/freedesktop/stereo/bell.oga)
       Both => trigger() + spawn
```

## Components

### 1. Indicator prominence

One-line constant change at `crates/vibeflow/src/render/tabs.rs:583`:

```rust
pub const INDICATOR_STRIPE_WIDTH_PX: u32 = 6; // was 3
```

`pulse_alpha` unchanged (still 0.4↔1.0 at 1.4 s). All `build_rects` math already references the constant; layout reflows automatically.

### 2. Bell config

Schema:

```rust
// config/schema.rs
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BellSection {
    pub mode: Option<String>,
    pub debounce_ms: Option<u64>,
}
// Plus: `pub bell: Option<BellSection>` in `ConfigFile`.
```

Resolved:

```rust
// config/mod.rs
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
            _ => Err(format!("unknown bell mode: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bell {
    pub mode: BellMode,
    pub debounce_ms: u64,
}
// Plus: `pub bell: Bell` in `Config`. Default: `Visual`, 100 ms.
```

`apply_bell` follows the Stage 11/12 pattern: log a `ConfigError` if mode-string is unrecognized; keep current value.

Audible bell impl (`render/bell.rs::play_audible_bell`):

```rust
pub fn play_audible_bell() {
    use std::process::{Command, Stdio};
    let result = Command::new("paplay")
        .arg("/usr/share/sounds/freedesktop/stereo/bell.oga")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = result {
        tracing::debug!("paplay not available; audible bell skipped: {e}");
    }
}
```

The Command spawns detached. We do NOT wait for it — bell ringing must not block the event loop. The file path `/usr/share/sounds/freedesktop/stereo/bell.oga` is the freedesktop standard; present on essentially all Linux desktops with sound. Fallback alternative for v0.1: nothing — if `paplay` isn't installed, the bell is silent (logged at debug). Future stages may add `canberra-gtk-play -i bell` as a second-chance fallback.

WindowApp gains:

```rust
    bell_mode: BellMode,
    bell_debounce: Duration,
    last_bell_at: Option<Instant>,
```

`handle_session_event(SessionEvent::Bell)` now dispatches as in the data-flow diagram.

### 3. iTerm2 theme system

#### ThemeData

```rust
// theme/mod.rs
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeData {
    pub name: String,
    pub ansi: [[f32; 4]; 16],
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub cursor: [f32; 4],
    pub cursor_text: [f32; 4],
    pub bold: Option<[f32; 4]>,
    pub link: Option<[f32; 4]>,
    pub selection: Option<[f32; 4]>,
}

impl ThemeData {
    /// Serialize to the TOML format vibeflow's registry stores.
    pub fn to_toml(&self) -> String;

    /// Deserialize from the TOML format vibeflow's registry stores.
    pub fn from_toml(s: &str) -> Result<Self, ThemeParseError>;
}
```

TOML format (`~/.config/vibeflow/themes/<name>.toml`):

```toml
# Solarized Dark — imported from solarized_dark.itermcolors on 2026-05-14
name = "solarized_dark"

[ansi]
ansi_0  = "#073642"   # black
ansi_1  = "#dc322f"   # red
ansi_2  = "#859900"   # green
ansi_3  = "#b58900"   # yellow
ansi_4  = "#268bd2"   # blue
ansi_5  = "#d33682"   # magenta
ansi_6  = "#2aa198"   # cyan
ansi_7  = "#eee8d5"   # white
ansi_8  = "#002b36"   # bright black
ansi_9  = "#cb4b16"   # bright red
ansi_10 = "#586e75"   # bright green
ansi_11 = "#657b83"   # bright yellow
ansi_12 = "#839496"   # bright blue
ansi_13 = "#6c71c4"   # bright magenta
ansi_14 = "#93a1a1"   # bright cyan
ansi_15 = "#fdf6e3"   # bright white

[special]
foreground  = "#839496"
background  = "#002b36"
cursor      = "#93a1a1"
cursor_text = "#002b36"
# bold, link, selection are optional
```

#### iTerm2 parser

```rust
// theme/iterm2.rs
use plist::Value;

#[derive(Debug)]
pub enum ItermImportError {
    NotAPlist(String),
    NotADict,
    MissingKey(String),
    BadColorValue(String),
}

pub fn parse_itermcolors(plist_bytes: &[u8]) -> Result<ThemeData, ItermImportError> {
    let value: Value = plist::from_bytes(plist_bytes)
        .map_err(|e| ItermImportError::NotAPlist(e.to_string()))?;
    let dict = value.into_dictionary().ok_or(ItermImportError::NotADict)?;

    // iTerm2's key format: "Ansi 0 Color", "Background Color", "Cursor Color", etc.
    // Each value is a sub-dict with Red/Green/Blue/Alpha Components keys, normalized 0.0-1.0.

    fn read_color(dict: &plist::Dictionary, key: &str) -> Result<[f32; 4], ItermImportError> {
        let sub = dict.get(key)
            .ok_or_else(|| ItermImportError::MissingKey(key.to_owned()))?
            .as_dictionary()
            .ok_or_else(|| ItermImportError::BadColorValue(key.to_owned()))?;
        let r = sub.get("Red Component").and_then(|v| v.as_real()).unwrap_or(0.0) as f32;
        let g = sub.get("Green Component").and_then(|v| v.as_real()).unwrap_or(0.0) as f32;
        let b = sub.get("Blue Component").and_then(|v| v.as_real()).unwrap_or(0.0) as f32;
        let a = sub.get("Alpha Component").and_then(|v| v.as_real()).unwrap_or(1.0) as f32;
        Ok([r, g, b, a])
    }

    let mut ansi = [[0.0f32; 4]; 16];
    for i in 0..16 {
        ansi[i] = read_color(&dict, &format!("Ansi {i} Color"))?;
    }
    let foreground = read_color(&dict, "Foreground Color")?;
    let background = read_color(&dict, "Background Color")?;
    let cursor = read_color(&dict, "Cursor Color")?;
    let cursor_text = read_color(&dict, "Cursor Text Color")?;
    let bold = read_color(&dict, "Bold Color").ok();
    let link = read_color(&dict, "Link Color").ok();
    let selection = read_color(&dict, "Selection Color").ok();

    Ok(ThemeData {
        name: String::new(),  // caller sets from filename basename
        ansi, foreground, background, cursor, cursor_text, bold, link, selection,
    })
}
```

#### CLI subcommand

`crates/vibeflow/src/main.rs`:

```rust
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // Parse subcommands BEFORE launching the GUI.
    if let Some(pos) = args.iter().position(|a| a == "--import-colors") {
        let Some(path_str) = args.get(pos + 1) else {
            eprintln!("usage: vibeflow --import-colors <path> [--overwrite]");
            return ExitCode::from(2);
        };
        let overwrite = args.iter().any(|a| a == "--overwrite");
        return run_import_colors(path_str, overwrite);
    }
    // ... existing main body (initialize tracing, run event loop, etc.)
}

fn run_import_colors(path_str: &str, overwrite: bool) -> ExitCode {
    use std::path::Path;
    let in_path = Path::new(path_str);
    let bytes = match std::fs::read(in_path) {
        Ok(b) => b,
        Err(e) => { eprintln!("cannot read {path_str}: {e}"); return ExitCode::from(1); }
    };
    let mut theme = match vibeflow::theme::iterm2::parse_itermcolors(&bytes) {
        Ok(t) => t,
        Err(e) => { eprintln!("parse error in {path_str}: {e:?}"); return ExitCode::from(1); }
    };
    // Derive theme name from input filename basename.
    let name = in_path.file_stem().and_then(|s| s.to_str()).unwrap_or("imported");
    theme.name = name.to_owned();

    let themes_dir = dirs::config_dir().unwrap_or_default().join("vibeflow/themes");
    if let Err(e) = std::fs::create_dir_all(&themes_dir) {
        eprintln!("cannot create {}: {e}", themes_dir.display());
        return ExitCode::from(1);
    }
    let out_path = themes_dir.join(format!("{name}.toml"));
    if out_path.exists() && !overwrite {
        eprintln!("theme '{name}' already exists at {}; use --overwrite to replace", out_path.display());
        return ExitCode::from(1);
    }
    let toml_str = theme.to_toml();
    if let Err(e) = std::fs::write(&out_path, toml_str) {
        eprintln!("cannot write {}: {e}", out_path.display());
        return ExitCode::from(1);
    }
    println!("imported theme '{name}' to {}", out_path.display());
    ExitCode::SUCCESS
}
```

Uses the `dirs` crate to resolve `~/.config/`. If `dirs` isn't already a dep, add it.

#### Theme registry

```rust
// theme/registry.rs
pub struct ThemeRegistry {
    themes: HashMap<String, ThemeData>,
    themes_dir: PathBuf,
}

impl ThemeRegistry {
    pub fn new_empty() -> Self { Self { themes: HashMap::new(), themes_dir: PathBuf::new() } }

    pub fn load(themes_dir: PathBuf) -> Self {
        let mut themes = HashMap::new();
        let Ok(entries) = std::fs::read_dir(&themes_dir) else {
            tracing::debug!("themes dir not found at {}; registry will be empty", themes_dir.display());
            return Self { themes, themes_dir };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") { continue; }
            let contents = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => { tracing::warn!("cannot read {}: {e}", path.display()); continue; }
            };
            let theme = match ThemeData::from_toml(&contents) {
                Ok(t) => t,
                Err(e) => { tracing::warn!("cannot parse theme {}: {e:?}", path.display()); continue; }
            };
            themes.insert(theme.name.clone(), theme);
        }
        Self { themes, themes_dir }
    }

    pub fn get(&self, name: &str) -> Option<&ThemeData> { self.themes.get(name) }
    pub fn names(&self) -> Vec<&str> { self.themes.keys().map(|s| s.as_str()).collect() }
    pub fn reload(&mut self) { *self = Self::load(self.themes_dir.clone()); }
}
```

#### Per-tab application

`PtySession.theme: Option<String>` field. Initialized at spawn from `App::default_theme`. `restart()` preserves.

```rust
// session/session.rs
impl PtySession {
    /// Stage 13: apply a named theme (or restore defaults if name is None).
    pub fn set_theme(&mut self, name: Option<String>, registry: &ThemeRegistry) {
        self.theme = name.clone();
        let Some(theme_name) = name else {
            // Restore Stage 9 hardcoded defaults.
            // alacritty_terminal::term::color::Colors::default() is the source of truth.
            *self.term.colors_mut() = alacritty_terminal::term::color::Colors::default();
            return;
        };
        let Some(theme) = registry.get(&theme_name) else {
            tracing::warn!("theme '{theme_name}' not found in registry; keeping current");
            return;
        };
        apply_theme_to_colors(self.term.colors_mut(), theme);
    }
}

fn apply_theme_to_colors(colors: &mut alacritty_terminal::term::color::Colors, theme: &ThemeData) {
    use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
    // Map theme.ansi[0..16] → colors[NamedColor::Black..NamedColor::BrightWhite].
    // Map theme.foreground → colors[NamedColor::Foreground]. Etc.
    // Color values: theme uses [f32; 4]; NamedColor uses Rgb { r, g, b }. Convert via *255.0.
    let to_rgb = |c: [f32; 4]| Rgb { r: (c[0] * 255.0) as u8, g: (c[1] * 255.0) as u8, b: (c[2] * 255.0) as u8 };
    // alacritty's Colors is indexed by NamedColor enum; for ansi 0..16 use Black, Red, ..., BrightWhite.
    let named_for_ansi = |i: usize| -> NamedColor {
        // Order: Black, Red, Green, Yellow, Blue, Magenta, Cyan, White,
        //        BrightBlack, BrightRed, ..., BrightWhite.
        match i {
            0 => NamedColor::Black, 1 => NamedColor::Red, 2 => NamedColor::Green, 3 => NamedColor::Yellow,
            4 => NamedColor::Blue, 5 => NamedColor::Magenta, 6 => NamedColor::Cyan, 7 => NamedColor::White,
            8 => NamedColor::BrightBlack, 9 => NamedColor::BrightRed, 10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow, 12 => NamedColor::BrightBlue, 13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan, 15 => NamedColor::BrightWhite,
            _ => NamedColor::Black,
        }
    };
    for i in 0..16 {
        colors[named_for_ansi(i)] = Some(to_rgb(theme.ansi[i]));
    }
    colors[NamedColor::Foreground] = Some(to_rgb(theme.foreground));
    colors[NamedColor::Background] = Some(to_rgb(theme.background));
    colors[NamedColor::Cursor] = Some(to_rgb(theme.cursor));
    colors[NamedColor::CursorText] = Some(to_rgb(theme.cursor_text));
    if let Some(b) = theme.bold { colors[NamedColor::Bold] = Some(to_rgb(b)); }
    // Link / selection don't map to NamedColor directly; renderer-level overrides handle these.
    // For v0.1, we ignore them in the term.colors_mut() write; future stages may wire to render::selection or hyperlink rendering.
}
```

(Verify `NamedColor` variant names + `Colors[NamedColor]` indexing API against `alacritty_terminal::term::color::*` during implementation. The senior pre-execution review catches API drift.)

#### Context menu integration

Stage 10's `tab_menu(target_idx, is_dead, tab_count, theme_names: &[String])` gets a new `theme_names` parameter (or accepts a `&ThemeRegistry` reference). For each theme name, append a `MenuItem` with `MenuAction::SetTheme(name.to_owned())`. Items appended after the existing tab menu items, after a separator:

```
Rename Tab
Restart Tab  (only when dead)
─── separator ───
New Tab
─── separator ───
Close Tab
Close Other Tabs
─── separator (if themes exist) ───
Theme: solarized_dark
Theme: gruvbox_dark
Theme: vibeflow_default
...
```

If no themes are loaded, the second separator + theme items are omitted.

`WindowApp::dispatch_menu_action(MenuAction::SetTheme(name), Some(idx))`: calls `self.app.tabs_mut()[idx].set_theme(Some(name), &self.theme_registry);` + redraw.

#### Config wiring

`[colors] preset = "..."` field added to existing `ColorsSection` (schema) and `Colors` (resolved). Default `None`. `apply_config`:

```rust
// in WindowApp::apply_config, after the existing [colors] propagation:
let new_preset = config.colors.preset.clone();
self.app.set_default_theme(new_preset.clone());
for s in self.app.tabs_mut().iter_mut() {
    s.set_theme(new_preset.clone(), &self.theme_registry);
}
```

This means every config reload reverts per-tab themes back to the global default. Acceptable trade-off documented in the brainstorm. Survives `PtySession::restart()` since `set_theme` writes to `term.colors_mut()` which the new `Term` doesn't preserve, but T6 of the implementation plan re-applies `self.theme` after restart.

### 4. Shift / Ctrl arrow keys

Append eight arms in `key_to_bytes` (window.rs:73). xterm-compatible escape sequences per the [keymap table](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-PC-Style-Function-Keys):

```rust
// In key_to_bytes, after existing arrow-key arms:
let ctrl_only = modifiers.control_key() && !modifiers.shift_key() && !modifiers.alt_key() && !modifiers.super_key();
let shift_only = modifiers.shift_key() && !modifiers.control_key() && !modifiers.alt_key() && !modifiers.super_key();

match logical_key {
    // Ctrl modifier (5 = ctrl in the modifier table):
    Key::Named(NamedKey::ArrowLeft) if ctrl_only => Some(b"\x1b[1;5D".to_vec()),
    Key::Named(NamedKey::ArrowRight) if ctrl_only => Some(b"\x1b[1;5C".to_vec()),
    Key::Named(NamedKey::ArrowUp) if ctrl_only => Some(b"\x1b[1;5A".to_vec()),
    Key::Named(NamedKey::ArrowDown) if ctrl_only => Some(b"\x1b[1;5B".to_vec()),

    // Shift modifier (2 = shift):
    Key::Named(NamedKey::ArrowLeft) if shift_only => Some(b"\x1b[1;2D".to_vec()),
    Key::Named(NamedKey::ArrowRight) if shift_only => Some(b"\x1b[1;2C".to_vec()),
    Key::Named(NamedKey::ArrowUp) if shift_only => Some(b"\x1b[1;2A".to_vec()),
    Key::Named(NamedKey::ArrowDown) if shift_only => Some(b"\x1b[1;2B".to_vec()),

    // ... existing arms continue
}
```

(Combined Ctrl+Shift modifier 6 / Alt modifier 3 deferred; rare.)

### 5. Block (column) selection

```rust
// render/selection.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Cell,
    Word,
    Line,
    Block,  // NEW
}
```

`mouse_down` signature gains `alt: bool` parameter. New selection starts with `mode = Block` iff `alt == true && !shift`.

`cells_in_range_block(start, end)` produces the rectangular set. `SelectionTracker::cells(term)` dispatches:

```rust
pub fn cells<'a>(&'a self, term: &'a Term<VoidListener>) -> Box<dyn Iterator<Item = Point> + 'a> {
    let Some(sel) = self.selection else { return Box::new(std::iter::empty()); };
    match sel.mode {
        SelectionMode::Block => Box::new(cells_in_range_block(sel.start, sel.end)),
        _ => Box::new(cells_in_range(sel.start, sel.end, term.columns())),
    }
}
```

`text()` for Block joins rows with `\n` (matches xterm/Alacritty convention).

Existing `build_selection_rects` (lifted-from-scrollback in Stage 12) doesn't need changes — it iterates `cells()` and renders rects per point. Block mode produces the correct cells; rects line up.

### 6. Shift-extend selection anchor

`mouse_down` body adjustment:

```rust
pub fn mouse_down(&mut self, point: Point, shift: bool, alt: bool, term: &Term<VoidListener>, now: Instant) {
    if shift {
        if let Some(sel) = self.selection.as_mut() {
            // Extend from existing anchor (start stays; end updates).
            sel.end = point;
            sel.mode = SelectionMode::Cell;  // shift-extend reverts to cell mode
            self.drag_anchor = Some(sel.start);
            self.snap_to_mode(term);
            return;
        }
    }
    // alt without shift starts block mode:
    let mode = if alt { SelectionMode::Block } else { SelectionMode::Cell };
    // ... existing fresh-selection path with `mode` instead of hardcoded Cell
}
```

`WindowApp` mouse handler passes `self.current_modifiers.alt_key()` to `mouse_down`.

### 7. Esc snap-to-bottom config knob

`[scrollback] snap_on_esc: bool = true` (default matches Stage 12). `WindowApp.snap_on_esc: bool` cache field.

In the existing Stage 12 snap-to-bottom hook in `window.rs`:

```rust
// Stage 12 + Stage 13: snap on input-producing keys, optionally exclude Esc.
if let Some(bytes) = key_to_bytes(&event.logical_key, event.state, modifiers) {
    // ...existing send_input + selection-clear...

    let is_esc = matches!(&event.logical_key, Key::Named(NamedKey::Escape));
    if !is_esc || self.snap_on_esc {
        let active_idx = self.app.active();
        if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
            if s.display_offset() > 0 {
                s.scroll_to_bottom(Instant::now());
            }
        }
    }
}
```

### 8. Font priority live-reload

`render/text_engine.rs::set_font_priorities` currently logs + invalidates the glyph cache. New behavior: also reorder the underlying `cosmic_text::FontSystem`'s `fontdb` so subsequent glyph requests find the new priority order.

cosmic-text 0.12 exposes `font_system.db_mut()` returning `&mut fontdb::Database`. Reordering steps:

1. Collect all face IDs currently in the db.
2. For each face, determine the priority score (lower index in the new priority list = higher priority).
3. Sort the database's face vector by score. **fontdb may not expose direct face-vector mutation.** If not, fall back to tearing down + rebuilding the entire `FontSystem` via `FontSystem::new()` and re-loading.

The teardown-rebuild fallback is acceptable for v0.1 since config reloads are user-initiated and infrequent. Cost: a few hundred ms one-time hitch on reload.

Verify fontdb 0.16's API during implementation. If the safe path (in-place reorder) is impractical, use the teardown approach.

## Edge cases

- **Theme registry empty**: context menu omits the theme submenu separator + items. Config-level `preset = "missing"` logs warn + keeps current theme (defaults).
- **Theme file malformed**: skipped at load; warn logged. Other valid themes still load.
- **CLI import: file already exists**: refuse with clear error unless `--overwrite`.
- **CLI import: malformed plist**: error message includes file path; non-zero exit.
- **Bell with `paplay` not installed**: `Command::spawn()` fails; logged at debug. Visual flash still fires if `mode = "both"`.
- **Bell debounce mid-mode-change**: changing mode mid-debounce doesn't reset the timer.
- **Shift+arrow snap-to-bottom**: arrow chord goes through `key_to_bytes` → snap fires (correct).
- **Block selection across scrollback + live**: Stage 12's display_offset translation in `build_selection_rects` handles it correctly.
- **`alt_key()` modifier on Wayland**: winit normalizes; should work like X11. Sanity-check in smoke walk.
- **Theme apply on dead tab**: harmless; `Term::colors_mut()` writes apply to the post-mortem grid. `restart_active` re-applies after restart.
- **Theme + `--import-colors --overwrite`**: refuses to overwrite the special name `"default"` (reserved for Stage 9 defaults). If user explicitly tries, error: "'default' is a reserved theme name."
- **Per-tab override reset by `apply_config`**: documented behavior. Editing config reverts ALL tabs to the new global preset.
- **Shift-extend with no existing selection**: falls through to fresh-selection path; same as Cell mode start.
- **Snap on Esc with `snap_on_esc = false`**: Esc still produces 0x1B byte to PTY; just doesn't snap viewport. All other input-producing keys still snap.
- **Font live-reload while glyphs are rendering**: the rebuild happens on the main thread (same as the existing `set_font_priorities`); the next frame uses the new state. Brief glyph-cache cold start re-renders all glyphs on first draw.

## Testing strategy

### Unit tests

- `theme::iterm2::parse_itermcolors`: parse a known-good `.itermcolors` fixture from `tests/fixtures/` (committed as part of T_iterm2 task); assert specific colors. Plus malformed-plist negative test.
- `theme::registry::ThemeRegistry::load`: scan a `tempfile::tempdir()` with valid + malformed `.toml` files; assert valid themes loaded, malformed skipped.
- `theme::ThemeData::to_toml` / `from_toml` round-trip: ensures the format we write is also parseable.
- `selection::cells_in_range_block`: table-driven cases for 3×3, 1×5, 5×1, single-cell, reversed (end < start).
- `selection::text` block mode: assert `\n`-joined rows.
- `selection::mouse_down` with shift + existing selection: anchor preserved.
- `selection::mouse_down` with alt: mode = Block.
- `key_to_bytes` for each of 8 new chords: assert exact byte sequence.
- `BellMode::from_str`: parses 4 valid + rejects unknown.
- `config::apply_bell`: bell mode parses; bad mode logs error.
- `config::apply_scrollback`: snap_on_esc default true; explicit override.

### Integration tests (`crates/vibeflow/tests/themes.rs`)

- `theme_preset_applies_at_spawn`: write a temp theme file to a temp themes dir; build a `ThemeRegistry::load`; create a `PtySession::spawn`; call `set_theme(Some(name), &registry)`; assert `term.colors()[NamedColor::Foreground]` equals the theme's foreground.
- `theme_set_via_menu_action`: scaffold the dispatch path; assert per-tab override applies to the right session and doesn't bleed across tabs.

### Manual smoke walk on VNC

1. **Indicator prominence**: open vibeflow, run `claude`, send prompt, observe `working` (blue 6 px stripe), wait for Claude to finish, observe `waiting` (amber 6 px stripe pulsing). Compare to Stage 12 — the stripe should now be obviously wider.
2. **Bell visual**: run `printf '\007'` in a shell. Window flashes.
3. **Bell silent**: edit config `[bell] mode = "silent"`. Save. Run `printf '\007'` — no flash.
4. **Bell audible**: edit config `[bell] mode = "audible"`. Save. Run `printf '\007'` — system bell sound. If `paplay` isn't installed, debug log shows the fallback; silent in practice.
5. **Bell debounce**: run `for i in 1 2 3 4 5; do printf '\007'; sleep 0.05; done`. With `debounce_ms = 100`, only the first bell fires; the rest are dropped.
6. **iTerm2 import**: download an .itermcolors file (e.g., from `iTerm2-Color-Schemes`). Run `./target/release/vibeflow --import-colors solarized_dark.itermcolors`. Verify `~/.config/vibeflow/themes/solarized_dark.toml` exists.
7. **Theme preset**: edit config `[colors] preset = "solarized_dark"`. Save. All tabs reflect the theme — background color changes immediately.
8. **Per-tab theme override**: right-click on a tab → click "Theme: gruvbox_dark" (or another available theme). That tab's colors change; other tabs unaffected.
9. **Theme + Stage 11 indicator**: with a custom theme applied, Waiting indicator still uses Stage 9's hardcoded amber (theme doesn't override Notice indicator colors — that's intentional).
10. **Ctrl+Right** in bash: jumps word-forward (assumes bash readline default with `forward-word` bound to `\e[1;5C`).
11. **Shift+Right** in bash: typically not bound; sends literal escape sequence. To verify in a more user-visible way, use `cat -v` and press Shift+Right — see `^[[1;2C` in the output.
12. **Block selection**: hold Alt and drag a rectangle across multiline text (e.g., `seq 1 50` output). Copy via right-click → "Copy". Paste elsewhere. Rectangular block joined by `\n`.
13. **Shift-extend selection**: drag a small selection. Hold Shift, click further away. Selection extends from the original anchor to the new click point.
14. **Esc snap config**: edit config `[scrollback] snap_on_esc = false`. Save. Scroll up. Press Esc. Viewport does NOT snap back (stays scrolled). Press a character — DOES snap.
15. **Font live-reload**: edit config `[fonts] priority = ["DejaVu Sans Mono", "Noto Color Emoji", "JetBrains Mono"]` (reverse from default). Save. Glyphs immediately re-render in the new font. Compare a recognizable character.

## Implementation sequencing (rough — refined in plan)

1. **Indicator prominence** — single constant bump.
2. **Esc snap config knob** — `[scrollback] snap_on_esc` schema + apply + gating.
3. **Bell mode + debounce** — `[bell]` schema + resolved + apply + `handle_session_event` dispatch + `play_audible_bell` helper.
4. **Shift / Ctrl arrow keys** — `key_to_bytes` extensions.
5. **Shift-extend selection anchor** — `mouse_down` refinement + test.
6. **Block selection** — `SelectionMode::Block` + `cells_in_range_block` + `mouse_down` alt parameter + `WindowApp` mouse handler pass-through.
7. **Font priority live-reload** — `text_engine` rebuild logic.
8. **Theme module scaffold** — `theme/mod.rs` + `ThemeData` types + TOML round-trip.
9. **iTerm2 parser** — `theme/iterm2.rs` + fixture-based tests.
10. **Theme registry** — `theme/registry.rs` + scan logic + reload.
11. **CLI subcommand** — `main.rs` `--import-colors` handler.
12. **Per-tab theme application** — `PtySession::set_theme` + `apply_theme_to_colors` + `Term::colors_mut()` integration.
13. **App default theme + propagation** — `App::set_default_theme` + `new_tab` + `restart_active`.
14. **Context menu Theme list** — `tab_menu` signature update + `MenuAction::SetTheme` + dispatch.
15. **`apply_config` wires `[colors] preset` + `[bell]` + `[scrollback] snap_on_esc`**.
16. **Integration tests** for theme + selection + bell.
17. **Senior pre-execution Sonnet review** (before T1).
18. **Manual smoke walk on VNC**.
19. **Senior holistic Sonnet review at end of stage**.

The first 7 tasks are all small and independent; the theme system (tasks 8-15) is the substantial work.

## Risks & mitigations

- **`alacritty_terminal::term::color::Colors` API**: NamedColor variants, `colors_mut()` accessor, `Colors[NamedColor]` indexing. Verify against `crates/vibeflow/src/render/colors.rs` (which already uses these patterns) + the crate docs. Senior pre-exec review catches drift.
- **`cosmic_text::FontSystem::db_mut`** for font reordering: fontdb may not allow in-place vector mutation. Plan picks one path at implementation time after reading fontdb 0.16 docs: either (a) reorder the face vector in place, or (b) tear down + rebuild `FontSystem` entirely on priority change. (b) is the safe fallback and acceptable for v0.1 since config reloads are infrequent.
- **plist crate panics or surprising parses**: trust the well-maintained crate; surface errors cleanly to user. Fixture test covers main happy path.
- **Per-tab theme reset on `apply_config`**: documented as known trade-off. Future stage adds persistent per-tab themes via tab-identity infrastructure.
- **Context menu theme list explosion**: if user imports 50 themes, the menu gets long. v0.2 should add submenus. For v0.1, the flat list at ~10 themes is tolerable.
- **`paplay` not installed**: silent failure with debug log. User-visible behavior: bell mode "audible" or "both" silently degrades to silent or visual respectively. Document in `integrations/README.md` extension or new bell config comment.
- **Block-selection rendering ambiguity**: when block selection crosses the cursor cell, the cursor highlight from Stage 6 might conflict with the selection rect. Likely no issue (both render; the latter on top). Verify in smoke walk.
- **Stage 13 scope creep**: 7-8 features in one stage. If any sub-feature surfaces unexpected complexity (especially theme module), defer it to a Stage 13b. Hard gates: indicator prominence + bell + arrow keys + Esc snap MUST ship; theme system + font live-reload + block selection are the "if-time-permits" cluster but all are within scope.

## Out-of-scope notes for future stages

- **Submenus for the context menu Theme list** (v0.2): the brainstorm flagged this. When user has >10 themes, a flat list becomes unwieldy. Submenu requires expansion to Stage 10's `ContextMenuState` overlay machinery — focus stacking, arrow-key navigation, parent-menu position tracking.
- **Persistent per-tab themes across vibeflow restart** (v0.2 or later): requires tab-identity tracking and per-tab state persistence to disk. Real design surface.
- **Audible bell on macOS / Windows**: outside v0.1 platform scope.
- **Color picker / theme editor UI** (post-v0.1): users edit theme TOML by hand for now.
- **Tier 2 wrapper shims** (`vibeflow-claude`, `vibeflow-codex`, `vibeflow-opencode`): separate stage; needs upstream tool prompt-pattern investigation. May land before or after Stage 14 ship.
