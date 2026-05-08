# vibeflow Stage 9 Design: TOML Config + Bundled UX Quick Wins

**Goal:** Add user-editable TOML configuration with hot-reload (shortcuts, colors, cursor blink, font priorities, PRIMARY clipboard), interactive tab rename, OSC 0 / OSC 2 title-setting from shells/AI tools, and arrow / navigation keys. Default vibeflow stays usable without writing any config; users opt in by creating `~/.config/vibeflow/config.toml`.

**Scope summary (locked):**
- TOML config at `~/.config/vibeflow/config.toml` (Linux XDG; macOS via `dirs::config_dir()`).
- Six configurable knob groups: shortcuts, colors (selection + 4 indicators), cursor blink, font priorities, PRIMARY clipboard, all optional.
- Hot-reload via the `notify` crate (file watcher thread → winit user-event → main thread distribution).
- Error handling: per-key tolerance + an in-window error banner listing dropped keys.
- Interactive tab rename via `Ctrl+Shift+E` / `F2` and right-click on a tab. Inline-in-tab editing UX.
- OSC 0 / OSC 2 (`\x1b]0;<title>\x07`) parsing → sets tab title from shell `$PROMPT_COMMAND` or AI tools.
- User rename takes precedence over OSC 0/2 (sticky until tab close / `Ctrl+Shift+R`).
- Arrow / nav keys (Up/Down/Left/Right/Home/End/PageUp/PageDown/Insert/Delete) → xterm ANSI sequences. Plain only — Shift / Ctrl modifier variants deferred.

**Out of scope (deferred to Stage 10+):**
- Color atlas `Rgba8UnormSrgb` migration (own phase).
- Full ANSI palette / theme (background, foreground, 16-color SGR, 256-color, truecolor).
- Bell behavior config (visual flash on/off, audio).
- Tab label rules per argv pattern (`[tab_label_rules]`).
- Right-click context menu for cell area (Copy/Paste menu) — needs overlay rendering subsystem.
- Block (column) selection (Alt+drag).
- Selection in scrollback / scrollback rendering.
- Shift / Ctrl-modifier arrow key variants.
- Selection that anchors to grid content across scroll.
- Shift-extend backward-anchor refinement.

---

## Architecture overview

### Module layout

| Path | Responsibility | Net delta |
|---|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Add `serde = { version = "1", features = ["derive"] }`, `toml = "0.8"`, `notify = "6"`, `dirs = "5"`. | +4 / 0 |
| `crates/vibeflow/src/lib.rs` (modify) | Declare `pub mod config;`. | +1 / 0 |
| `crates/vibeflow/src/config/mod.rs` (create) | `Config` aggregate; `load(path) -> (Config, Vec<ConfigError>)`; `default()` impl; color hex parser; shortcut spec parser. | +280 |
| `crates/vibeflow/src/config/schema.rs` (create) | `serde::Deserialize`-derived types: `ConfigFile`, `ColorsSection`, `CursorSection`, `FontsSection`, `ClipboardSection`, `ShortcutsSection`. All fields `Option<T>`. | +120 |
| `crates/vibeflow/src/config/watcher.rs` (create) | `spawn(path, proxy)` — starts the `notify` thread with 250 ms debounce; sends `AppUserEvent::ConfigReloaded` / `ConfigError` via `EventLoopProxy`. | +120 |
| `crates/vibeflow/src/config/error_banner.rs` (create) | `ErrorBannerState { errors: Vec<ConfigError>, dismissed: bool }`; render into the unified rect+glyph buffer (one rect range, one glyph range). | +130 |
| `crates/vibeflow/src/keymap.rs` (modify) | Replace hard-coded `match_shortcut` with a `ShortcutTable` consumed from `Config.shortcuts`. Lookup: `(Key, ModifiersState) -> Option<Shortcut>`. | +120 / -100 |
| `crates/vibeflow/src/render/mod.rs` (modify) | Setters: `set_selection_color`, `set_indicator_colors`, `set_cursor_blink_ms`, `set_font_priorities`. Banner rect/glyph slot in unified buffer. | +90 / -10 |
| `crates/vibeflow/src/render/text_engine.rs` (modify) | `set_font_priorities` reorders the cosmic-text fontdb fallback chain. | +30 |
| `crates/vibeflow/src/render/cursor.rs` (modify) | `set_blink_ms(ms: u64)`; `0` disables blink (cursor always visible). | +10 |
| `crates/vibeflow/src/render/tabs.rs` (modify) | `push_text_glyphs` honors `Option<&RenameInputState>` — substitutes buffer + cursor caret for the renaming tab. | +60 / -5 |
| `crates/vibeflow/src/clipboard.rs` (modify) | `set_primary_enabled(bool)`; `copy` and `paste` write/read PRIMARY when enabled (Linux). | +50 / -5 |
| `crates/vibeflow/src/session/osc.rs` (modify) | Add `DispatchEvent::SetTitle(String)`; parse OSC 0 / OSC 2 prefixes. | +60 |
| `crates/vibeflow/src/session/session.rs` (modify) | `pub user_renamed: bool` on `PtySession`; `set_title_from_osc(title)` honoring sticky flag; reset `user_renamed` on `restart()`. | +25 |
| `crates/vibeflow/src/window.rs` (modify) | `EventLoop::<AppUserEvent>::with_user_event()`; receive `ConfigReloaded` and `ConfigError`; `RenameInputState` + start/commit/cancel handlers; right-click→TabBody triggers rename; arrow / nav keys in `key_to_bytes`. | +280 / -20 |
| `crates/vibeflow/src/main.rs` (modify) | Event loop type swap to `EventLoop::<AppUserEvent>`; spawn watcher thread post-`resumed`. | +20 / -5 |
| `docs/TESTING.md` (modify) | Append Stage 9 manual smoke checklist. | +60 |

**Net add:** ~+1480 / −145 (≈ +1335 net), 13 files modified, 4 files created, 4 deps added.

### Threading model

Unchanged from Stage 8 except for one new background thread:

- **Main thread:** all rendering, all UI state, `Config` ownership, `WindowApp` event handling.
- **PTY reader threads (one per session):** unchanged from Stage 1+.
- **Watcher thread (NEW):** owns a `notify::RecommendedWatcher` and an `EventLoopProxy<AppUserEvent>`. Wakes on file-modify events, debounces 250 ms, parses `~/.config/vibeflow/config.toml`, sends `AppUserEvent::ConfigReloaded { config, errors }` (or `ConfigError(io_error)`) back to the main thread.

The watcher thread holds NO shared mutable state — it produces immutable values and ships them via the proxy.

### Data flow

```
  ~/.config/vibeflow/config.toml
            │
            │ inotify / kqueue
            ▼
  ┌─────────────────────┐                ┌──────────────────────────────────┐
  │  watcher thread     │  send_event    │  WindowApp (main thread)         │
  │  - notify watcher   │ ─────────────► │                                  │
  │  - 250ms debounce   │                │  user_event(ConfigReloaded)      │
  │  - parse + validate │                │    ├─► renderer.set_*()          │
  └─────────────────────┘                │    ├─► self.shortcuts = …       │
                                          │    ├─► self.clipboard.set_*()  │
                                          │    └─► error_banner.update(…)  │
                                          └──────────────────────────────────┘
```

OSC 0 / OSC 2 and arrow keys are independent paths in `OscDispatcher` and `key_to_bytes` — no Config dependency.

---

## Config schema

`~/.config/vibeflow/config.toml`:

```toml
# vibeflow Stage 9 — all keys optional; missing keys use built-in defaults.

[shortcuts]
# action = list-of-keys (any listed key triggers the action).
# Format: "ctrl+shift+t" / "super+t" / "ctrl+alt+t" / "ctrl+tab" / "f2".
# Modifiers (lowercased): ctrl, shift, alt, super.
# Built-in defaults (do not need to be repeated unless you want to remove one):
# new_tab     = ["ctrl+shift+t", "super+t"]
# close_tab   = ["ctrl+shift+w", "super+w"]
# next_tab    = ["ctrl+tab",     "super+tab"]
# prev_tab    = ["ctrl+shift+tab", "super+shift+tab"]
# restart_tab = ["ctrl+shift+r", "super+r"]
# copy        = ["ctrl+shift+c", "super+c"]
# paste       = ["ctrl+shift+v", "super+v"]
# rename_tab  = ["ctrl+shift+e", "f2"]

# Example overrides for VNC-from-Mac users (Super doesn't reach vibeflow):
# new_tab    = ["ctrl+shift+t", "ctrl+alt+t"]
# copy       = ["ctrl+shift+c", "ctrl+alt+c"]

[colors]
# RGBA hex. Format: "#RRGGBBAA" (8 hex digits + leading #).
selection          = "#6699ff66"  # 40% alpha blue (current Stage 8)
indicator_active   = "#22cc66ff"  # green
indicator_working  = "#3399ffff"  # blue
indicator_waiting  = "#ffaa00ff"  # amber
indicator_inactive = "#888888ff"  # grey

[cursor]
# Milliseconds per full blink cycle (off→on→off). 0 = no blink.
blink_ms = 500

[fonts]
# Ordered priority list. Earlier entries take precedence in cosmic-text's
# fallback chain. Names match what fontconfig / fc-list reports.
priority = [
  "JetBrains Mono",
  "Noto Color Emoji",       # earlier than DejaVu fixes the smiley emoji issue
  "DejaVu Sans Mono",
]

[clipboard]
# Also write/read the X11 PRIMARY selection (middle-click paste).
# Silently ignored on non-Linux.
primary = true
```

### Schema rules

- All keys optional. Missing keys → built-in defaults. Missing top-level sections → defaults for that whole section.
- Color format `#RRGGBBAA` (8 hex digits, leading `#`, lowercase or uppercase). 6-digit `#RRGGBB` is rejected (must specify alpha) and falls back per-key.
- Shortcut tokens: `+`-separated, lowercase or any case (parser normalizes). Modifiers: `ctrl`, `shift`, `alt`, `super`. Keys: lowercase letters (`a`–`z`), `tab`, `f1`–`f12`. Unknown tokens warn-and-drop the entire entry for that action.
- `cursor.blink_ms = 0` disables blinking entirely (cursor renders solid).
- `fonts.priority` is the entire fallback chain — vibeflow does NOT auto-append defaults. To keep `JetBrains Mono` and add an override, the user re-states the full list.
- `clipboard.primary` silently no-ops on non-Linux platforms.

### Built-in defaults (for reference)

```rust
Config {
    shortcuts: ShortcutTable::with_default_bindings(),
    colors: ColorsSection {
        selection: rgba(0x66, 0x99, 0xFF, 0x66),
        indicator_active:   rgba(0x22, 0xCC, 0x66, 0xFF),
        indicator_working:  rgba(0x33, 0x99, 0xFF, 0xFF),
        indicator_waiting:  rgba(0xFF, 0xAA, 0x00, 0xFF),
        indicator_inactive: rgba(0x88, 0x88, 0x88, 0xFF),
    },
    cursor: CursorSection { blink_ms: 500 },
    fonts: FontsSection {
        priority: vec![
            "JetBrains Mono".to_string(),
            "Noto Color Emoji".to_string(),
            "DejaVu Sans Mono".to_string(),
        ],
    },
    clipboard: ClipboardSection { primary: true },
}
```

---

## Hot-reload + error handling

### Startup flow

1. `Config::load(path) -> (Config, Vec<ConfigError>)`:
   - File missing → built-in defaults, no errors.
   - File present but unreadable (perm error / IO error) → defaults + a single `ConfigError::IoError(io::Error)`.
   - File present, valid TOML → per-key tolerant parsing: bad keys collected into `Vec<ConfigError>`, valid keys applied.
   - File present, invalid TOML (syntax error from the `toml` crate) → defaults + a single `ConfigError::Syntax { line, col, msg }`.
2. Renderer / WindowApp / clipboard initialize from the loaded `Config`.
3. After `ApplicationHandler::resumed` fires, `WindowApp` spawns the watcher thread.

### Watcher thread

```rust
pub fn spawn(
    path: PathBuf,
    proxy: EventLoopProxy<AppUserEvent>,
) -> notify::Result<JoinHandle<()>> {
    // Implementation sketch:
    // - Build a notify::RecommendedWatcher with a 250ms debouncer.
    // - On Modify / Create event, re-read + re-parse the file.
    // - On Remove event, send AppUserEvent::ConfigError("config file removed").
    //   Keep current Config in the main thread; do NOT revert to defaults.
    //   Re-add the watch on the next Create event.
    // - Send AppUserEvent::ConfigReloaded { config, errors } via the proxy.
}
```

The thread shuts down naturally when the `EventLoopProxy` is dropped (window close → main thread exits → proxy goes away → next `send_event` errors → thread exits its loop).

### Main-thread reception (`WindowApp::user_event`)

```rust
fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppUserEvent) {
    match event {
        AppUserEvent::ConfigReloaded { config, errors } => {
            self.renderer.as_mut().map(|r| {
                r.set_selection_color(config.colors.selection);
                r.set_indicator_colors(config.colors.indicator_table());
                r.set_cursor_blink_ms(config.cursor.blink_ms);
                r.set_font_priorities(config.fonts.priority.clone());
            });
            self.shortcuts = config.shortcuts.clone();
            self.clipboard.as_mut().map(|c| c.set_primary_enabled(config.clipboard.primary));
            self.error_banner = if errors.is_empty() {
                None
            } else {
                Some(ErrorBannerState::new(errors))
            };
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        AppUserEvent::ConfigError(err) => {
            // Single-error path (file removed, IO error). Keep current Config.
            self.error_banner = Some(ErrorBannerState::new(vec![err]));
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }
}
```

### Error banner rendering

`ErrorBannerState`:
```rust
pub struct ErrorBannerState {
    errors: Vec<ConfigError>,
    /// True after the user pressed Esc; banner stays gone until next reload.
    dismissed: bool,
}

pub enum ConfigError {
    Syntax { line: usize, col: usize, msg: String },
    IoError(io::Error),
    InvalidColor { key: String, value: String, msg: String },
    InvalidShortcut { action: String, value: String, msg: String },
    UnknownKey { path: String },
    InvalidValue { key: String, expected: String, got: String },
}
```

Render:
- Floats over the top of the cell area (does NOT shrink the PTY grid — keeps the shell unaware).
- Semi-transparent dark-red rect (`rgba(0.4, 0.1, 0.1, 0.85)`), white glyph color.
- Text: `"⚠ N config keys ignored: <first error short-form>; press Esc to dismiss"`. If N > 1, append `…(more)` and log details to `tracing::warn`.
- Reuses the unified rect + glyph buffer pattern from Stage 7.5 / 8: one new rect range slot (`error_banner_rect_offset..error_banner_rect_offset + 1`), one new glyph range slot.
- `Esc` keypress while no rename is active sets `dismissed = true`, banner clears.
- Auto-clears on the next `ConfigReloaded` whose `errors.is_empty()`.

### Edge cases

| Scenario | Behavior |
|---|---|
| File missing at startup | Silent defaults, no banner. |
| File deleted at runtime | Banner: "config file removed; using last loaded values". Current Config retained. Watcher rebinds on next Create. |
| Atomic write (vim `:wq`) | Debounce coalesces Remove + Create into one Reload. |
| Permission error (chmod 000) | Banner: "permission denied: ~/.config/vibeflow/config.toml". |
| TOML syntax error | Banner: "syntax error at L42:C8". Current Config retained. |
| Single bad key (`cursor.blink_ms = "fast"`) | Per-key tolerance: that key falls back to default; valid keys still apply. Banner lists it. |
| All keys bad | Defaults applied; banner lists them; vibeflow stays usable. |

---

## Tab rename + OSC 0/2

### Data model

`PtySession` gains:
```rust
pub struct PtySession {
    // ... existing fields
    /// True once the user has manually renamed via Ctrl+Shift+E or right-click.
    /// Sticky for the life of this session — subsequent OSC 0/2 are ignored.
    /// Cleared on Ctrl+Shift+R restart (since restart respawns the session).
    pub user_renamed: bool,
}
```

### OSC 0 / OSC 2 parsing in `OscDispatcher`

New event variant:
```rust
pub enum DispatchEvent {
    AiState(Frame),               // OSC 1338 — existing
    Prompt(PromptMarker),         // OSC 133 — existing
    SetTitle(String),             // OSC 0 / OSC 2 — NEW
    PassThrough(Vec<u8>),         // existing
}
```

Body inspection rules (in `OscDispatcher::flush_osc`):
- Body starts with `0;` → `DispatchEvent::SetTitle(rest)` (OSC 0 sets both window title + icon name; we treat the payload as the title).
- Body starts with `2;` → `DispatchEvent::SetTitle(rest)`.
- Body starts with `1;` (icon name only) → ignored / pass-through.
- Existing 1338 / 133 paths unchanged.

OSC 0/2 has only one parameter — embedded `;` characters in the title are part of the title (e.g. `\x1b]0;a;b\x07` → `SetTitle("a;b")`).

Title bytes are decoded as UTF-8 lossily (replace invalid sequences with U+FFFD). 1024-byte cap (matches xterm) — longer titles get truncated.

### Routing in `PtySession::poll`

```rust
DispatchEvent::SetTitle(title) => {
    if !self.user_renamed {
        self.label.title = title;
        // subtitle continues to be tracker-driven; refresh_default_subtitle
        // is independent and runs on tracker state changes.
    }
    // else: silently dropped — user wins
}
```

### Interactive rename state on `WindowApp`

```rust
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

self.rename_state: Option<RenameInputState>
```

### Rename triggers

- `Shortcut::RenameTab` (default keys: `ctrl+shift+e`, `f2`) → `start_rename(self.app.active())`.
- Right-click on tab body (`MouseButton::Right`, Released, in `TabBarHit::TabBody(idx)`) → `start_rename(idx)`.

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
```

### Rename keyboard handling

In `WindowEvent::KeyboardInput`, when `rename_state.is_some()`:
- Bypass `match_shortcut` AND `key_to_bytes` — the rename input fully owns the keyboard.
- `Enter` → commit: `tabs[idx].label.title = buffer.clone()`, `tabs[idx].user_renamed = true`, `rename_state = None`.
- `Esc` → cancel: `tabs[idx].label.title = original`, `rename_state = None`.
- `Backspace` → delete the grapheme before `cursor_pos` (use `unicode-segmentation` — already a transitive dep via cosmic-text).
- `Delete` → delete the grapheme at `cursor_pos`.
- `Left` / `Right` arrow → move `cursor_pos` by one grapheme.
- `Home` → `cursor_pos = 0`.
- `End` → `cursor_pos = buffer.len()`.
- Plain Character key → insert at `cursor_pos`, advance.
- Modifier-only keys (Ctrl/Shift/Alt/Super alone) → ignored.
- Empty `buffer` on Enter is allowed (commits to "" — legitimate "blank tab title" choice).

### Rename mouse handling

While `rename_state.is_some()`:
- Click on the same tab being renamed → no-op (let user click without losing edits).
- Click on any other tab → cancel rename (restore `original`), then process the click normally (which switches active tab).
- Click in cell area → cancel rename, then process the click normally (which starts a selection).
- Click on `+` (new-tab) → cancel rename, then spawn the new tab.
- Click on `×` (close) → cancel rename, then close the target tab.

### Render override (in `tabs.rs::push_text_glyphs`)

Signature change:
```rust
pub fn push_text_glyphs(
    // ... existing parameters
    rename_state: Option<&RenameInputState>,
)
```

If `rename_state` is Some and `rename_state.tab_idx == this_tab`:
- Render `&rename_state.buffer` instead of `session.label().title`.
- Apply a tinted underlay rect (the title row, tinted `rgba(1.0, 1.0, 1.0, 0.15)`) so the user sees the editing affordance.
- Render a blinking caret (`█`, half-cell width) at `rename_state.cursor_pos`. Reuse the existing cursor-blink phase tracking from Stage 7.

### Edge cases

| Scenario | Behavior |
|---|---|
| Tab closed during rename (other tab) | `rename_state` retained if the renaming tab still exists; cleared if the renaming tab itself was closed. |
| Tab being renamed gets `restart()` | `rename_state` cleared (the session was respawned). |
| Resize during rename | `rename_state` retained — it's tab-strip state, not cell-grid state. |
| Selection drag in cell area + start rename | Selection persists; rename takes keyboard. Click-out-cancels-rename rule applies. |
| Empty string commit | Allowed; tab title becomes "". |
| Buffer wider than tab | Stage 6's existing ellipsis truncation in `push_text_glyphs` clips. |
| Multi-byte characters (CJK, emoji) | `cursor_pos` always at a grapheme boundary; arrow keys / Backspace / Delete operate on graphemes via `unicode-segmentation`. |

### Precedence rules

| Source | When applied |
|---|---|
| `argv0` basename | initial default at `PtySession::spawn` |
| OSC 0 / OSC 2 from shell or AI tool | replaces title iff `user_renamed == false` |
| User rename (`Ctrl+Shift+E` / `F2` / right-click) | sets `user_renamed = true`; sticky until tab close / `Ctrl+Shift+R` restart |

Subtitle (line 2) is always tracker-driven from OSC 1338 state — never user-set. Stage 9 does NOT change subtitle handling.

---

## Arrow / nav key sequences

New `match` arms in `key_to_bytes` (in `window.rs`), all with `modifiers == empty()`. xterm-compatible:

| Key | Bytes | Why |
|---|---|---|
| `Up` | `\x1b[A` | bash `previous-history` |
| `Down` | `\x1b[B` | bash `next-history` |
| `Right` | `\x1b[C` | bash `forward-char`, vim move |
| `Left` | `\x1b[D` | bash `backward-char`, vim move |
| `Home` | `\x1b[H` | bash `beginning-of-line` |
| `End` | `\x1b[F` | bash `end-of-line` |
| `PageUp` | `\x1b[5~` | less `back-line` |
| `PageDown` | `\x1b[6~` | less `forward-line` |
| `Insert` | `\x1b[2~` | rare but stable |
| `Delete` | `\x1b[3~` | bash `delete-char` |

Modifier-bearing variants (Shift+Arrow, Ctrl+Arrow, etc.) are deferred. xterm's full encoding (`CSI 1; <mod> <key>`) is large; deferring to Stage 10/11 keeps Stage 9 focused.

---

## Testing

### Unit tests (~46 new)

| Module | Count | What |
|---|---|---|
| `config/schema.rs` | 12 | parse full + partial + empty configs; per-knob defaults; per-knob malformed (color, shortcut, blink_ms) → ConfigError collected, valid keys still apply |
| `config/mod.rs` | 3 | hex color parser (8-digit ok, 6-digit rejected, 3-digit rejected, alpha-channel correctness); shortcut spec parser (`ctrl+shift+t`, unknown modifier warn-drop); IO error handling |
| `config/watcher.rs` | 1 (`#[ignore]`) | end-to-end with a `tempdir` — write, modify, assert reload event delivered. Ignored due to OS-level timing. |
| `session/osc.rs` | 4 | OSC 0 → SetTitle; OSC 2 → SetTitle; OSC 1 ignored; OSC 0 with embedded `;` |
| `session/session.rs` | 3 | OSC 0 sets title when not user_renamed; OSC 0 dropped when user_renamed; restart clears user_renamed |
| `keymap.rs` | 5 | ShortcutTable::from_config consumes Config.shortcuts; new defaults match Stage 8 hard-coded behavior; user table overrides; conflicting binding takes the first; empty action list disables |
| `window.rs` (rename) | 6 | start_rename initializes; Backspace deletes graphemes (test multi-byte); Enter commits + sets user_renamed; Esc cancels + restores; click-out cancels; arrow keys move cursor |
| `window.rs` (key_to_bytes) | 10 | one per new arrow / nav key |
| `render/mod.rs` (banner) | 2 | ErrorBannerState::new; banner rect generation |

After Stage 8's 176 default + 13 ignored, Stage 9 lands at ~222 default + 14 ignored.

### Integration test

`crates/vibeflow/tests/config_reload.rs` (`#[ignore]`, X11-required) — full end-to-end: launch vibeflow, write a config with a known selection_color, wait for reload, dump renderer state via a debug-flag, assert the color updated. Defer to manual smoke if too brittle on CI.

### Manual smoke (Stage 9 section in `docs/TESTING.md`)

- Cold-start with no config file → defaults work.
- Write `config.toml` with `selection_color = "#ff0000ff"` → reload → selection rect turns red.
- Write `cursor.blink_ms = 0` → cursor stops blinking.
- Write malformed config (`blink_ms = "fast"`) → red banner appears with the error; defaults applied. Fix → banner disappears.
- Delete config file at runtime → banner warns, current values retained.
- Recreate config file → banner clears, new values applied.
- Custom shortcut: rebind `new_tab` to `["ctrl+alt+t"]` → `Ctrl+Shift+T` no longer spawns tab; `Ctrl+Alt+T` does.
- Tab rename: `Ctrl+Shift+E` → tab title becomes editable, type "claude work", Enter → tab now reads "claude work" / "active". `F2` also works.
- Right-click another tab → its title becomes editable.
- Esc during rename → reverts to original.
- Click outside the tab being renamed → cancels rename.
- After user rename, run a script that emits `\x1b]0;new_title\x07` → the renamed tab is unaffected (sticky).
- A regular shell prompt that emits OSC 0 (e.g. `PS1='\[\e]0;\u@\h: \w\a\]$ '`) → un-renamed tabs pick up the title.
- Up arrow at bash prompt → previous command appears. Down arrow → next.
- Page Up / Page Down in `less` → paging works.
- Middle-click paste from another app (Linux) when `clipboard.primary = true` → pastes. With `primary = false` → does nothing.
- Restart Ctrl+Shift+R after a user rename → fresh session, tab title back to default ("bash").

### Cross-cutting

- `cargo +nightly fuzz run parse -- -max_total_time=60` on the protocol parser — should still pass (no protocol changes).
- `cargo doc -- -D warnings` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.

---

## Risks / open questions

1. **`notify` crate event-debounce reliability across filesystems.** Vim's atomic-rename produces Remove + Create on most filesystems but Modify on some (e.g. when `:set nobackup`). The 250ms debouncer + a Modify-or-Create trigger covers both. If a CI runner uses an unusual fs, the watcher integration test may need to be loosened to `#[ignore]`-only.

2. **Cosmic-text fontdb reordering.** `set_font_priorities` recreates the cosmic-text fontdb fallback chain. If the user-supplied list omits a font that the rendered text needs (e.g. removes Noto Color Emoji while having emoji content on screen), rasterization falls through to the system fallback chain. Should still work but may render emoji as tofu. Not a Stage 9 bug — document in the manual smoke.

3. **`unicode-segmentation` for grapheme-aware rename.** May or may not already be a transitive dep through cosmic-text. If absent, adding it is a small one-dep cost (no transitive deps). Verify before relying on it.

4. **PRIMARY clipboard on Wayland.** PRIMARY is X11-specific; Wayland's protocol differs (`zwlr_data_control_v1`). `arboard` 3.6 supports both via different code paths; the `primary` flag should map cleanly. If smoke shows it doesn't, either fall back to "PRIMARY only on X11" or open an upstream issue.

5. **Reload of the ShortcutTable mid-rename.** If config reloads while a rename is in progress, the old `Shortcut::RenameTab` mapping is stale. Probably fine — the rename state machine doesn't read `self.shortcuts`. But document.

6. **Banner Z-order with selection rendering.** The error banner is drawn between tab bar and selection rects, OR after the selection rects. The latter means selection highlights "show through" the banner if they overlap; the former hides them. For Stage 9 the banner is drawn AFTER the cells but BEFORE the dead-tab banner — same layer as selection. This is acceptable; selection inside the banner area gets visually obscured but the data is fine.

7. **Default fallback when a user re-states only some font priorities.** Spec says `fonts.priority` is the entire fallback chain. If the user writes `priority = ["Iosevka"]`, they lose emoji / fallback fonts. Document this as a footgun in the Stage 9 smoke section.

8. **Writing tests for the watcher requires a real filesystem and inotify wakeups.** Time-sensitive. The single watcher integration test is `#[ignore]`-only; manual smoke covers the practical path.

These risks are addressed by the senior pre-execution review of this plan and Stage 9 manual smoke.

---

## Spec coverage check

Mapping requirements → implementation surface:

| Requirement | Covered by |
|---|---|
| TOML config file at XDG path | `config/mod.rs` + `dirs` crate |
| `serde`-derived schema | `config/schema.rs` |
| Per-key tolerance | `config/mod.rs::load` |
| Hot-reload via file watcher | `config/watcher.rs` + `notify` crate |
| Error banner on bad config | `config/error_banner.rs` + `render/mod.rs` |
| Configurable shortcuts | `config/schema.rs::ShortcutsSection` + `keymap.rs::ShortcutTable` |
| Configurable colors (5) | `config/schema.rs::ColorsSection` + renderer setters |
| Configurable cursor blink | `config/schema.rs::CursorSection` + `render/cursor.rs::set_blink_ms` |
| Configurable font priorities | `config/schema.rs::FontsSection` + `render/text_engine.rs::set_font_priorities` |
| PRIMARY clipboard toggle | `config/schema.rs::ClipboardSection` + `clipboard.rs::set_primary_enabled` |
| Arrow / nav keys | `window.rs::key_to_bytes` |
| Interactive tab rename (Ctrl+Shift+E, F2, right-click) | `window.rs::RenameInputState`, `tabs.rs::push_text_glyphs` |
| OSC 0 / OSC 2 title-setting | `session/osc.rs::DispatchEvent::SetTitle` + `session/session.rs` |
| User rename precedence | `session/session.rs::user_renamed` flag |

Every spec item maps to a concrete code surface. No placeholders, no TBDs.
