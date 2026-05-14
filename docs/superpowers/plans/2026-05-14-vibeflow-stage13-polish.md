# vibeflow Stage 13 — Polish bucket Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 8 polish features in one stage — indicator prominence bump, bell mode config + audible bell, iTerm2 theme system (CLI + registry + per-tab override), modifier arrow keys, block selection, shift-extend anchor, Esc snap config knob, font live-reload.

**Architecture:** Theme system is the only meaty piece — a new `theme/` module with `ThemeData` + iTerm2 plist parser + filesystem-backed registry, plus per-tab `Term::colors_mut()` application via context menu. Everything else is small and independent: single-constant bumps, config additions following Stage 11/12 patterns, `key_to_bytes` extensions, `SelectionMode::Block` variant.

**Tech Stack:** Rust, winit 0.30, wgpu, alacritty_terminal 0.24 (`Term::colors_mut`, `NamedColor`), cosmic-text 0.12 (FontSystem db_mut), plist crate 1.x (NEW dep), dirs crate (for `~/.config/` resolution).

**Spec:** `docs/superpowers/specs/2026-05-14-vibeflow-stage13-polish-design.md`

---

## Critical Stage 13 safety guards (re-state in every implementer dispatch prompt)

1. **DO NOT delete or weaken any existing test.** Function-name diff before reporting DONE:
   ```
   git show HEAD~1:<file> | grep -E '^\s*fn ' > /tmp/pre.txt
   git show HEAD:<file>   | grep -E '^\s*fn ' > /tmp/post.txt
   diff /tmp/pre.txt /tmp/post.txt
   ```
2. **Report deviations honestly.**
3. **Cargo from `/home/bhengen/dev/vibeflow`.** No `cd` into crate dirs.
4. **Quality gate per task:** `cargo fmt --all`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`. All four green before commit.

## Pre-execution senior review (workflow step, not a task)

Before T1 dispatch, run a Sonnet `general-purpose` review against actual source. Reviewer prompt sketch:

> Read `docs/superpowers/plans/2026-05-14-vibeflow-stage13-polish.md` end-to-end. Read the source files it modifies, plus `alacritty_terminal-0.24.2/src/term/color.rs` (specifically the `Colors` struct and `NamedColor` enum — Stage 13 writes via `term.colors_mut()`), `cosmic-text-0.12.1/src/font/system.rs` (specifically `FontSystem::db_mut`), `plist-1.x/src/lib.rs` (verify `plist::from_bytes` and `Value::as_dictionary` API). Categorize findings as Critical / Important / Minor / Verified-correct. Apply Critical fixes before T1 dispatch.

Stages 10/11/12 reviews each caught 5-10 compile-blockers. Worth ~30 minutes.

---

## File structure

| File | Status | Used by |
|---|---|---|
| `crates/vibeflow/src/theme/mod.rs` | NEW | T10, T14 |
| `crates/vibeflow/src/theme/iterm2.rs` | NEW | T11, T13 |
| `crates/vibeflow/src/theme/registry.rs` | NEW | T12, T14, T16, T17 |
| `crates/vibeflow/src/lib.rs` | TOUCHED | T10 (`pub mod theme;`) |
| `crates/vibeflow/src/main.rs` | TOUCHED | T13 (`--import-colors` subcommand) |
| `crates/vibeflow/Cargo.toml` | TOUCHED | T11 (`plist = "1"`), T13 (`dirs = "5"` if not present) |
| `crates/vibeflow/src/render/tabs.rs:583` | TOUCHED (1 line) | T1 |
| `crates/vibeflow/src/render/bell.rs` | TOUCHED | T4 (`play_audible_bell` helper) |
| `crates/vibeflow/src/render/selection.rs` | TOUCHED | T6 (shift-extend), T7 (Block mode), T8 (`mouse_down` alt param) |
| `crates/vibeflow/src/render/text_engine.rs` | TOUCHED | T9 (font live-reload) |
| `crates/vibeflow/src/render/context_menu.rs` | TOUCHED | T16 (Theme list + `MenuAction::SetTheme`) |
| `crates/vibeflow/src/session/session.rs` | TOUCHED | T14 (`PtySession.theme`, `set_theme`) |
| `crates/vibeflow/src/app.rs` | TOUCHED | T15 (`default_theme` field + setter + new_tab/restart propagation) |
| `crates/vibeflow/src/window.rs` | TOUCHED | T2, T4, T5, T8, T16 dispatch, T17 |
| `crates/vibeflow/src/config/schema.rs` | TOUCHED | T2, T3, T17 (`BellSection`, `ScrollbackSection.snap_on_esc`, `ColorsSection.preset`) |
| `crates/vibeflow/src/config/mod.rs` | TOUCHED | T2, T3, T17 (resolved structs + defaults + apply) |
| `crates/vibeflow/tests/themes.rs` | NEW | T18 (integration tests) |

---

### Task 1: Indicator prominence — bump stripe width

**Files:**
- Modify: `crates/vibeflow/src/render/tabs.rs:583`

The single-line fix Stage 11 smoke walk asked for. Cell-render math already references the constant; no other change needed.

- [ ] **Step 1: Apply the change.**

Edit `crates/vibeflow/src/render/tabs.rs` line 583:
```rust
pub const INDICATOR_STRIPE_WIDTH_PX: u32 = 6;
```
(was `3`)

- [ ] **Step 2: Quality gate.**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: all green. No existing test pins the literal value 3 (verified by `grep -n "stripe.*3\|INDICATOR.*3" crates/vibeflow/`).

- [ ] **Step 3: Commit.**

```bash
git add crates/vibeflow/src/render/tabs.rs
git commit -m "feat(stage13): indicator stripe width 3 → 6 px"
```

---

### Task 2: `[scrollback] snap_on_esc` config knob (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs`
- Modify: `crates/vibeflow/src/config/mod.rs`
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Add tests to config/schema.rs::tests.**

Append:
```rust
    #[test]
    fn scrollback_snap_on_esc_field_parses() {
        let toml = r#"
[scrollback]
snap_on_esc = false
"#;
        let cs: super::ConfigFile = toml::from_str(toml).expect("parse");
        let sb = cs.scrollback.expect("scrollback present");
        assert_eq!(sb.snap_on_esc, Some(false));
    }
```

- [ ] **Step 2: Run; expect compile error.**

```bash
cargo test --package vibeflow --lib config::schema::tests::scrollback_snap_on_esc 2>&1 | tail -10
```
Expected: `no field 'snap_on_esc'`.

- [ ] **Step 3: Add `snap_on_esc: Option<bool>` to `ScrollbackSection`.**

In the existing `ScrollbackSection` struct (Stage 12 added it), append:
```rust
    pub snap_on_esc: Option<bool>,
```

- [ ] **Step 4: Add tests to config/mod.rs::tests.**

Append:
```rust
    #[test]
    fn scrollback_snap_on_esc_defaults_true() {
        let cf = Config::default_values();
        assert!(cf.scrollback.snap_on_esc);
    }

    #[test]
    fn scrollback_snap_on_esc_load_overrides() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
[scrollback]
snap_on_esc = false
"#).expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert!(!cf.scrollback.snap_on_esc);
    }
```

- [ ] **Step 5: Add `snap_on_esc: bool` to resolved `Scrollback`.**

In `config/mod.rs`'s `Scrollback` struct (Stage 12 added it):
```rust
    pub snap_on_esc: bool,
```

In `Config::default_values()`'s `scrollback:` literal, add the field:
```rust
            snap_on_esc: true,
```
(Add this alongside the existing `history_lines`, `wheel_lines_per_detent`, `scrollbar_fade_ms`.)

In `apply_scrollback(schema, resolved)`:
```rust
    if let Some(v) = schema.snap_on_esc {
        resolved.snap_on_esc = v;
    }
```

- [ ] **Step 6: Run tests.**

```bash
cargo test --package vibeflow --lib config 2>&1 | tail -10
```
Expected: 3 new tests pass.

- [ ] **Step 7: Wire into window.rs (gate the Stage 12 Esc snap).**

In `window.rs`, find the Stage 12 snap-to-bottom hook (it lives inside the `if let Some(bytes) = key_to_bytes(...)` branch). Grep:
```bash
grep -n "scroll_to_bottom\|display_offset() > 0" crates/vibeflow/src/window.rs | head -10
```

Add a `snap_on_esc: bool` field to `WindowApp` near the other Stage 12 caches:
```rust
    /// Stage 13: mirror of `Config.scrollback.snap_on_esc`. When false, Esc
    /// does NOT snap to bottom (only character-producing keys do).
    snap_on_esc: bool,
```

Initialize in `WindowApp::new`:
```rust
            snap_on_esc: true,
```

Modify the snap hook to gate on Esc:
```rust
// Stage 12 + Stage 13: snap on input-producing keys, optionally exclude Esc.
let is_esc = matches!(&event.logical_key, Key::Named(NamedKey::Escape));
if !is_esc || self.snap_on_esc {
    let active_idx = self.app.active();
    if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
        if s.display_offset() > 0 {
            s.scroll_to_bottom(Instant::now());
            if let Some(w) = self.window.as_ref() { w.request_redraw(); }
        }
    }
}
```

(T17 wires `apply_config` to update `self.snap_on_esc` on reload.)

- [ ] **Step 8: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/config/schema.rs crates/vibeflow/src/config/mod.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage13): [scrollback] snap_on_esc config knob (default true)"
```

---

### Task 3: `[bell]` config — schema + resolved + apply (TDD)

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs`
- Modify: `crates/vibeflow/src/config/mod.rs`

- [ ] **Step 1: Add tests to config/schema.rs::tests.**

```rust
    #[test]
    fn bell_section_parses_all_fields() {
        let toml = r#"
[bell]
mode = "audible"
debounce_ms = 200
"#;
        let cs: super::ConfigFile = toml::from_str(toml).expect("parse");
        let b = cs.bell.expect("bell present");
        assert_eq!(b.mode.as_deref(), Some("audible"));
        assert_eq!(b.debounce_ms, Some(200));
    }

    #[test]
    fn bell_section_missing_keeps_none() {
        let cs: super::ConfigFile = toml::from_str("").expect("parse");
        assert!(cs.bell.is_none());
    }

    #[test]
    fn bell_section_rejects_unknown_field() {
        let toml = r#"
[bell]
bogus = 1
"#;
        let r: Result<super::ConfigFile, _> = toml::from_str(toml);
        assert!(r.is_err());
    }
```

- [ ] **Step 2: Run; expect compile error.**

```bash
cargo test --package vibeflow --lib config::schema::tests::bell 2>&1 | tail -10
```
Expected: `cannot find type 'BellSection'` or `field 'bell' does not exist`.

- [ ] **Step 3: Add `BellSection` + `bell` field on `ConfigFile`.**

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BellSection {
    pub mode: Option<String>,
    pub debounce_ms: Option<u64>,
}
```

In `ConfigFile`:
```rust
    pub bell: Option<BellSection>,
```

- [ ] **Step 4: Add resolved `Bell` + `BellMode` + tests to mod.rs.**

In `config/mod.rs`:
```rust
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
```

Add `pub bell: Bell` to `Config`. In `default_values()`:
```rust
            bell: Bell {
                mode: BellMode::Visual,
                debounce_ms: 100,
            },
```

Add `apply_bell` helper:
```rust
fn apply_bell(schema: schema::BellSection, resolved: &mut Bell, errors: &mut Vec<ConfigError>) {
    if let Some(m) = schema.mode {
        match m.parse::<BellMode>() {
            Ok(mode) => resolved.mode = mode,
            Err(e) => errors.push(ConfigError::new("bell.mode", &e)),
        }
    }
    if let Some(v) = schema.debounce_ms {
        resolved.debounce_ms = v;
    }
}
```

Match the existing `apply_*` signature pattern — `ConfigError::new` takes a key path + message. Read `apply_colors` for the exact constructor shape.

Wire `apply_bell` into `Config::load`:
```rust
        if let Some(b) = file.bell {
            apply_bell(b, &mut defaults.bell, &mut errors);
        }
```

Tests in `config/mod.rs::tests`:
```rust
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
        std::fs::write(&path, r#"
[bell]
mode = "audible"
debounce_ms = 50
"#).expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(cf.bell.mode, BellMode::Audible);
        assert_eq!(cf.bell.debounce_ms, 50);
    }

    #[test]
    fn bell_mode_invalid_string_logs_error_keeps_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"
[bell]
mode = "blinking-lights"
"#).expect("write");
        let (cf, errors) = Config::load(&path);
        assert!(!errors.is_empty(), "expected error for invalid mode");
        assert_eq!(cf.bell.mode, BellMode::Visual); // default preserved
    }
```

- [ ] **Step 5: Run tests.**

```bash
cargo test --package vibeflow --lib config 2>&1 | tail -10
```
Expected: 6 new tests pass (3 schema + 3 mod).

- [ ] **Step 6: Quality gate + commit.**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/config/schema.rs crates/vibeflow/src/config/mod.rs
git commit -m "feat(stage13): [bell] config — mode (4 variants) + debounce_ms"
```

---

### Task 4: Bell mode dispatch + audible bell helper

**Files:**
- Modify: `crates/vibeflow/src/render/bell.rs`
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Add `play_audible_bell` helper in `render/bell.rs`.**

Append (outside the existing `BellFlash` impl):

```rust
/// Stage 13: play the system bell sound via `paplay`. Spawned detached;
/// never blocks the event loop. If `paplay` isn't installed or the sound
/// file is missing, logs at debug level and silently degrades.
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

- [ ] **Step 2: Add `bell_mode`, `bell_debounce`, `last_bell_at` cache fields on WindowApp.**

In `WindowApp` struct (near other Stage 11/12 caches):
```rust
    /// Stage 13: bell behavior config cache.
    bell_mode: crate::config::BellMode,
    bell_debounce: std::time::Duration,
    last_bell_at: Option<std::time::Instant>,
```

In `WindowApp::new`:
```rust
            bell_mode: crate::config::BellMode::Visual,
            bell_debounce: std::time::Duration::from_millis(100),
            last_bell_at: None,
```

- [ ] **Step 3: Replace the existing `SessionEvent::Bell` arm in `handle_session_event`.**

Find it (currently at `window.rs:328`):
```rust
            SessionEvent::Bell => {
                tracing::trace!(tab = idx, "bell rung");
                // Only flash for the active tab to avoid background tabs spamming.
                if idx == self.app.active() {
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.note_bell();
                    }
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
```

Replace with:
```rust
            SessionEvent::Bell => {
                tracing::trace!(tab = idx, "bell rung");
                // Stage 13: only react if this is the active tab.
                if idx != self.app.active() {
                    return;
                }
                // Debounce — drop bells closer than `bell_debounce`.
                let now = std::time::Instant::now();
                if let Some(last) = self.last_bell_at {
                    if now.saturating_duration_since(last) < self.bell_debounce {
                        return;
                    }
                }
                self.last_bell_at = Some(now);

                use crate::config::BellMode;
                match self.bell_mode {
                    BellMode::Silent => {}
                    BellMode::Visual => {
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.note_bell();
                        }
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                    BellMode::Audible => {
                        crate::render::bell::play_audible_bell();
                    }
                    BellMode::Both => {
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.note_bell();
                        }
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                        crate::render::bell::play_audible_bell();
                    }
                }
            }
```

(T17 wires `apply_config` to update `bell_mode` and `bell_debounce`.)

- [ ] **Step 4: Quality gate + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/render/bell.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage13): bell mode dispatch + paplay audible helper + debounce"
```

---

### Task 5: Shift / Ctrl modifier arrow keys (TDD)

**Files:**
- Modify: `crates/vibeflow/src/window.rs` (`key_to_bytes` function)

- [ ] **Step 1: Add tests in `window.rs::tests`.**

Find the existing key_to_bytes tests via:
```bash
grep -n "key_to_bytes\|mod tests" crates/vibeflow/src/window.rs | head -10
```

Append in the tests module:
```rust
    use winit::keyboard::{Key, NamedKey, ModifiersState};

    fn mods_ctrl() -> ModifiersState { ModifiersState::CONTROL }
    fn mods_shift() -> ModifiersState { ModifiersState::SHIFT }

    #[test]
    fn ctrl_arrows_emit_xterm_modifier_5_sequences() {
        let pressed = winit::event::ElementState::Pressed;
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowLeft), pressed, mods_ctrl()),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowRight), pressed, mods_ctrl()),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowUp), pressed, mods_ctrl()),
            Some(b"\x1b[1;5A".to_vec())
        );
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowDown), pressed, mods_ctrl()),
            Some(b"\x1b[1;5B".to_vec())
        );
    }

    #[test]
    fn shift_arrows_emit_xterm_modifier_2_sequences() {
        let pressed = winit::event::ElementState::Pressed;
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowLeft), pressed, mods_shift()),
            Some(b"\x1b[1;2D".to_vec())
        );
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowRight), pressed, mods_shift()),
            Some(b"\x1b[1;2C".to_vec())
        );
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowUp), pressed, mods_shift()),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowDown), pressed, mods_shift()),
            Some(b"\x1b[1;2B".to_vec())
        );
    }

    #[test]
    fn plain_arrows_still_emit_unmodified_sequences() {
        let pressed = winit::event::ElementState::Pressed;
        let none = ModifiersState::empty();
        // Stage 8 baseline: plain arrows emit \x1b[ABCD per xterm.
        assert_eq!(
            super::key_to_bytes(&Key::Named(NamedKey::ArrowLeft), pressed, none),
            Some(b"\x1b[D".to_vec())
        );
    }
```

- [ ] **Step 2: Run; expect failures.**

```bash
cargo test --package vibeflow --lib tests::ctrl_arrows 2>&1 | tail -10
cargo test --package vibeflow --lib tests::shift_arrows 2>&1 | tail -10
```
Expected: failures (current code returns the plain sequences regardless of modifier, or returns None for some).

- [ ] **Step 3: Add the modifier-arm match arms in `key_to_bytes`.**

In `key_to_bytes`, locate the existing arrow-key arms (Stage 8). Before them, add the modifier-specific arms:

```rust
        // Stage 13: Ctrl + arrow keys (xterm modifier code 5).
        Key::Named(NamedKey::ArrowLeft)
            if modifiers.control_key()
                && !modifiers.shift_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;5D".to_vec())
        }
        Key::Named(NamedKey::ArrowRight)
            if modifiers.control_key()
                && !modifiers.shift_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;5C".to_vec())
        }
        Key::Named(NamedKey::ArrowUp)
            if modifiers.control_key()
                && !modifiers.shift_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;5A".to_vec())
        }
        Key::Named(NamedKey::ArrowDown)
            if modifiers.control_key()
                && !modifiers.shift_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;5B".to_vec())
        }

        // Stage 13: Shift + arrow keys (xterm modifier code 2).
        Key::Named(NamedKey::ArrowLeft)
            if modifiers.shift_key()
                && !modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;2D".to_vec())
        }
        Key::Named(NamedKey::ArrowRight)
            if modifiers.shift_key()
                && !modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;2C".to_vec())
        }
        Key::Named(NamedKey::ArrowUp)
            if modifiers.shift_key()
                && !modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;2A".to_vec())
        }
        Key::Named(NamedKey::ArrowDown)
            if modifiers.shift_key()
                && !modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
        {
            Some(b"\x1b[1;2B".to_vec())
        }
```

Match arms before the existing unmodified ones so they take precedence.

- [ ] **Step 4: Run tests; expect pass.**

```bash
cargo test --package vibeflow --lib tests::ctrl_arrows 2>&1 | tail -5
cargo test --package vibeflow --lib tests::shift_arrows 2>&1 | tail -5
cargo test --package vibeflow --lib tests::plain_arrows 2>&1 | tail -5
```
Expected: all 3 pass.

- [ ] **Step 5: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage13): key_to_bytes — Ctrl/Shift modifier arrow keys (xterm seq)"
```

---

### Task 6: Shift-extend selection anchor (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/selection.rs`

- [ ] **Step 1: Add test in `selection::tests`.**

```rust
    #[test]
    fn shift_click_with_existing_selection_extends_anchor() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        // First: establish a selection from (5, 3) to (5, 8).
        t.mouse_down(pt(5, 3), false, false, &term, now);
        t.mouse_drag(pt(5, 8), &term);
        t.mouse_up();
        let initial = t.current().expect("initial selection");
        assert_eq!(initial.start, pt(5, 3));
        assert_eq!(initial.end, pt(5, 8));

        // Now: shift-click at (5, 15). Expect anchor at (5, 3), end at (5, 15).
        t.mouse_down(pt(5, 15), true, false, &term, now);
        let extended = t.current().expect("extended selection");
        assert_eq!(extended.start, pt(5, 3), "anchor should not move on shift-extend");
        assert_eq!(extended.end, pt(5, 15));
    }

    #[test]
    fn shift_click_with_no_existing_selection_starts_fresh() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        // Shift-click with no prior selection should behave like a regular click.
        t.mouse_down(pt(5, 3), true, false, &term, now);
        let sel = t.current().expect("selection set");
        assert_eq!(sel.start, pt(5, 3));
        assert_eq!(sel.end, pt(5, 3));
    }
```

These tests assume the signature `mouse_down(point, shift, alt, term, now)` (4-arg with `alt`). Task 7 adds the `alt` parameter; this task's tests pass `false` for `alt`. If T6 is dispatched before T7, the signature won't have `alt` yet — adapt the test arguments OR dispatch T7's signature change FIRST. Recommend: dispatch T6 + T7 as a single combined task, OR have T6 add the `alt` parameter and T7 fill in Block-mode logic. Following the latter approach: T6's `mouse_down` signature gets `alt: bool` (unused for now, threaded through); T7 adds Block-mode semantics.

- [ ] **Step 2: Add `alt: bool` to `SelectionTracker::mouse_down` signature.**

Current (Stage 8/10):
```rust
pub fn mouse_down(&mut self, point: Point, shift: bool, term: &Term<VoidListener>, now: Instant)
```

New:
```rust
pub fn mouse_down(&mut self, point: Point, shift: bool, alt: bool, term: &Term<VoidListener>, now: Instant)
```

Update all callers. Find them:
```bash
grep -rn "selection.mouse_down\|\.mouse_down(" crates/vibeflow/src/window.rs crates/vibeflow/src/session/session.rs 2>&1 | head -10
```

For each caller, insert `false` as the new `alt` argument until T8 wires the actual value from `current_modifiers.alt_key()`. Existing test callers similarly: add `false`.

- [ ] **Step 3: Run; expect failures (shift-extend logic not implemented yet).**

```bash
cargo test --package vibeflow --lib render::selection::tests::shift_click 2>&1 | tail -10
```

- [ ] **Step 4: Modify `mouse_down` body to handle shift-extend.**

```rust
pub fn mouse_down(&mut self, point: Point, shift: bool, alt: bool, term: &Term<VoidListener>, now: Instant) {
    // Stage 13: shift-click extends existing anchor instead of starting fresh.
    if shift {
        if let Some(sel) = self.selection.as_mut() {
            sel.end = point;
            sel.mode = SelectionMode::Cell;
            self.drag_anchor = Some(sel.start);
            self.snap_to_mode(term);
            return;
        }
        // No existing selection — fall through to fresh-selection path.
    }

    // ...existing body unchanged. Note: `alt` is threaded through but unused
    // until Task 7 adds Block-mode semantics.
    let _ = alt;
    // (rest of existing body)
}
```

If the existing body uses `let mode = SelectionMode::Cell;`, T7 will replace that with `let mode = if alt { SelectionMode::Block } else { SelectionMode::Cell };`. For T6, leave the body as-is plus the new shift-extend prelude.

- [ ] **Step 5: Run; expect pass.**

```bash
cargo test --package vibeflow --lib render::selection::tests::shift_click 2>&1 | tail -10
```
Expected: 2 passed.

- [ ] **Step 6: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -10
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/render/selection.rs crates/vibeflow/src/window.rs crates/vibeflow/src/session/session.rs
git commit -m "feat(stage13): shift-extend selection anchor + threaded alt param"
```

(Adjust the `git add` list based on which files needed caller updates.)

---

### Task 7: Block (column) selection (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/selection.rs`

- [ ] **Step 1: Add tests in `selection::tests`.**

```rust
    #[test]
    fn cells_in_range_block_yields_rectangle() {
        // 3x3 block from (1, 2) to (3, 4): 3 rows × 3 cols = 9 cells.
        let cells: Vec<_> = cells_in_range_block(pt(1, 2), pt(3, 4)).collect();
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0], pt(1, 2));
        assert_eq!(cells[2], pt(1, 4));
        assert_eq!(cells[3], pt(2, 2));
        assert_eq!(cells[8], pt(3, 4));
    }

    #[test]
    fn cells_in_range_block_handles_reverse_order() {
        // Bottom-right to top-left: same set of cells, normalized.
        let cells: Vec<_> = cells_in_range_block(pt(3, 4), pt(1, 2)).collect();
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0], pt(1, 2));
    }

    #[test]
    fn cells_in_range_block_single_cell() {
        let cells: Vec<_> = cells_in_range_block(pt(5, 5), pt(5, 5)).collect();
        assert_eq!(cells, vec![pt(5, 5)]);
    }

    #[test]
    fn alt_drag_sets_block_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(2, 3), false, true, &term, now);  // alt = true
        t.mouse_drag(pt(5, 7), &term);
        t.mouse_up();
        let sel = t.current().expect("selection");
        assert_eq!(sel.mode, SelectionMode::Block);
    }

    #[test]
    fn block_text_joins_rows_with_newline() {
        // Construct a Term where rows have predictable content via direct grid writes.
        // Simplest path: feed bytes through a dispatcher. Defer to integration test
        // if direct construction is too fiddly.
        // For the unit test, just verify cells iteration yields block-shape Points.
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(0, 0), false, true, &term, now);
        t.mouse_drag(pt(1, 2), &term);  // 2 rows × 3 cols
        t.mouse_up();
        let cells_collected: Vec<_> = t.cells(&term).collect();
        assert_eq!(cells_collected.len(), 6);
    }
```

- [ ] **Step 2: Run; expect compile error (`cells_in_range_block` not found, `SelectionMode::Block` not found).**

```bash
cargo test --package vibeflow --lib render::selection::tests::cells_in_range_block 2>&1 | tail -10
```

- [ ] **Step 3: Add `SelectionMode::Block` variant.**

In the `SelectionMode` enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Cell,
    Word,
    Line,
    Block,  // NEW
}
```

- [ ] **Step 4: Add `cells_in_range_block` function.**

In `selection.rs` (alongside the existing `cells_in_range`):
```rust
/// Stage 13: rectangular cell iteration for block-selection mode.
/// Normalizes start/end so any pair of corners works.
pub fn cells_in_range_block(start: Point, end: Point) -> impl Iterator<Item = Point> {
    use alacritty_terminal::index::{Column, Line};
    let (top, bottom) = if start.line.0 <= end.line.0 {
        (start.line.0, end.line.0)
    } else {
        (end.line.0, start.line.0)
    };
    let (left, right) = if start.column.0 <= end.column.0 {
        (start.column.0, end.column.0)
    } else {
        (end.column.0, start.column.0)
    };
    (top..=bottom).flat_map(move |line| {
        (left..=right).map(move |col| Point::new(Line(line), Column(col)))
    })
}
```

- [ ] **Step 5: Update `SelectionTracker::cells` to dispatch on mode.**

```rust
pub fn cells<'a>(&'a self, term: &'a Term<VoidListener>) -> Box<dyn Iterator<Item = Point> + 'a> {
    let Some(sel) = self.selection else {
        return Box::new(std::iter::empty());
    };
    match sel.mode {
        SelectionMode::Block => Box::new(cells_in_range_block(sel.start, sel.end)),
        _ => Box::new(cells_in_range(sel.start, sel.end, term.columns())),
    }
}
```

- [ ] **Step 6: Update `SelectionTracker::text` to emit `\n`-joined rows for Block.**

```rust
pub fn text(&self, term: &Term<VoidListener>) -> Option<String> {
    let sel = self.selection?;
    let mut out = String::new();
    let mut current_line = sel.start.line;  // (or .min — block normalizes)
    let cells_iter: Box<dyn Iterator<Item = Point>> = match sel.mode {
        SelectionMode::Block => {
            // For block, walk rows in normalized order.
            current_line = sel.start.line.min(sel.end.line);
            Box::new(cells_in_range_block(sel.start, sel.end))
        }
        _ => Box::new(cells_in_range(sel.start, sel.end, term.columns())),
    };
    for p in cells_iter {
        if p.line != current_line {
            out.push('\n');
            current_line = p.line;
        }
        let cell = &term.grid()[p];
        out.push(cell.c);
    }
    Some(sel.mode == SelectionMode::Block).map(|_| out.clone());  // (no-op; keep `out`)
    Some(out)
}
```

(The redundant `Some(...)` line above is a typo cleanup target for the implementer — just return `Some(out)`.)

- [ ] **Step 7: Make `mouse_down` start Block mode on `alt && !shift`.**

In `mouse_down` body (continuing from Task 6's threaded `alt: bool`):
```rust
pub fn mouse_down(&mut self, point: Point, shift: bool, alt: bool, term: &Term<VoidListener>, now: Instant) {
    // Stage 13 shift-extend (Task 6 — keep as-is):
    if shift {
        if let Some(sel) = self.selection.as_mut() {
            sel.end = point;
            sel.mode = SelectionMode::Cell;
            self.drag_anchor = Some(sel.start);
            self.snap_to_mode(term);
            return;
        }
    }

    // Stage 13 Task 7: alt+drag enters Block mode.
    let initial_mode = if alt && !shift {
        SelectionMode::Block
    } else {
        SelectionMode::Cell  // existing default; word/line modes promote on multi-click
    };

    // ...rest of existing body, but replace `SelectionMode::Cell` literal where
    // selection is constructed with `initial_mode`.
}
```

Read the existing `mouse_down` body and find the `SelectionMode::Cell` literal that's used at selection-creation time. Replace with `initial_mode`. The multi-click promotion path (single → word → line) should NOT apply when `alt` is true — block mode should be sticky. If the existing code conditionally promotes mode based on `bump_click`, gate that promotion on `!alt` to preserve Block.

- [ ] **Step 8: Run tests; expect pass.**

```bash
cargo test --package vibeflow --lib render::selection::tests::cells_in_range_block 2>&1 | tail -10
cargo test --package vibeflow --lib render::selection::tests::alt_drag 2>&1 | tail -5
cargo test --package vibeflow --lib render::selection::tests::block_text 2>&1 | tail -5
```

- [ ] **Step 9: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/render/selection.rs
git commit -m "feat(stage13): SelectionMode::Block + alt+drag for column selection"
```

---

### Task 8: WindowApp mouse handler passes `alt` to mouse_down

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Find the existing left-click mouse_down call.**

```bash
grep -n "selection.mouse_down" crates/vibeflow/src/window.rs
```

- [ ] **Step 2: Update the call site to pass `current_modifiers.alt_key()`.**

Current call (Stage 8/10):
```rust
s.selection.mouse_down(point, self.current_modifiers.shift_key(), s.term(), now);
```

Updated:
```rust
s.selection.mouse_down(
    point,
    self.current_modifiers.shift_key(),
    self.current_modifiers.alt_key(),
    s.term(),
    now,
);
```

- [ ] **Step 3: Build + smoke check on VNC.**

```bash
cargo build --release 2>&1 | tail -3
pkill -f 'target/release/vibeflow' 2>/dev/null
DISPLAY=:1 RUST_LOG=vibeflow=info ./target/release/vibeflow > /tmp/vf-t8.log 2>&1 &
sleep 2
pgrep -f 'target/release/vibeflow' && echo "✓ alive" || echo "✗ died"
```

Manual: hold Alt + drag in a tab. Block-shape selection should appear. Stop the binary.

- [ ] **Step 4: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/window.rs
git commit -m "feat(stage13): WindowApp mouse handler passes alt_key() to mouse_down"
```

---

### Task 9: Font priority live-reload

**Files:**
- Modify: `crates/vibeflow/src/render/text_engine.rs`

The current `set_font_priorities` logs + invalidates the glyph cache but the underlying `cosmic_text::FontSystem`'s font database is unchanged — the new order applies only on restart.

- [ ] **Step 1: Read existing `set_font_priorities`.**

```bash
grep -n "fn set_font_priorities\|FontSystem\|db_mut\|fontdb" crates/vibeflow/src/render/text_engine.rs | head -15
```

Stage 13 path: rebuild the `FontSystem` from scratch with the new priority. The fallback is simpler and avoids fontdb 0.16 in-place mutation gymnastics.

- [ ] **Step 2: Modify `set_font_priorities` to rebuild `FontSystem`.**

The existing body invalidates the glyph cache. Add tear-down + rebuild:

```rust
pub fn set_font_priorities(&mut self, priority: Vec<String>) {
    tracing::info!("font priorities updated (rebuild)" /* existing log */;
                   priority = ?priority);
    // Stage 13: rebuild FontSystem so the new priority order is reflected
    // in fontdb's lookup. Cheaper alternative (in-place reorder via
    // db_mut()) is fontdb-version-fragile; rebuild is simple and config
    // reloads are rare.
    self.font_priority = priority.clone();
    self.font_system = cosmic_text::FontSystem::new();
    // Invalidate glyph cache as before.
    self.atlases.clear();
    // ... existing cache-invalidation logic
}
```

Adapt to the actual existing field names. Read the file:
```bash
sed -n '1,80p' crates/vibeflow/src/render/text_engine.rs
```

The function might already invalidate caches; add only the `font_system = FontSystem::new()` line. The `font_priority` field assignment is already there.

`FontSystem::new()` scans system fonts on construction — takes ~300ms first time. On subsequent rebuilds, it's faster (fontdb caches some metadata). Acceptable for rare config reloads.

- [ ] **Step 3: Run tests + workspace.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```

- [ ] **Step 4: Commit.**

```bash
git add crates/vibeflow/src/render/text_engine.rs
git commit -m "feat(stage13): font priority live-reload via FontSystem rebuild"
```

---

### Task 10: `theme` module scaffold (TDD)

**Files:**
- Create: `crates/vibeflow/src/theme/mod.rs`
- Modify: `crates/vibeflow/src/lib.rs`

- [ ] **Step 1: Add `pub mod theme;` to `lib.rs`.**

In `crates/vibeflow/src/lib.rs`, alongside existing `pub mod *;` declarations, alphabetically between `session` and `window` — actually `theme` < `window` so insert just before `window`:
```rust
pub mod theme;
```

- [ ] **Step 2: Create `crates/vibeflow/src/theme/mod.rs` with `ThemeData` + sub-mod declarations.**

```rust
//! Stage 13: theme registry + iTerm2 color-scheme import.
//!
//! User imports `.itermcolors` files via `vibeflow --import-colors <path>`.
//! Themes land in `~/.config/vibeflow/themes/<name>.toml`. At startup,
//! `ThemeRegistry::load` scans the directory. `[colors] preset = "name"`
//! selects the default. Per-tab override via Stage 10's context menu.

pub mod iterm2;
pub mod registry;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeData {
    pub name: String,
    pub ansi: [[f32; 4]; 16],
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub cursor: [f32; 4],
    pub cursor_text: [f32; 4],
    #[serde(default)]
    pub bold: Option<[f32; 4]>,
    #[serde(default)]
    pub link: Option<[f32; 4]>,
    #[serde(default)]
    pub selection: Option<[f32; 4]>,
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeParseError {
    #[error("invalid TOML: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("invalid hex color in field {field}: {value}")]
    BadHex { field: String, value: String },
}

impl ThemeData {
    /// Stage 13: serialize to the on-disk TOML format.
    pub fn to_toml(&self) -> String {
        // Convert [f32; 4] colors to "#rrggbb" strings.
        fn hex(c: [f32; 4]) -> String {
            format!(
                "#{:02x}{:02x}{:02x}",
                (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                (c[2].clamp(0.0, 1.0) * 255.0) as u8,
            )
        }
        let mut out = format!("name = \"{}\"\n\n[ansi]\n", self.name);
        for (i, c) in self.ansi.iter().enumerate() {
            out.push_str(&format!("ansi_{} = \"{}\"\n", i, hex(*c)));
        }
        out.push_str(&format!("\n[special]\nforeground = \"{}\"\n", hex(self.foreground)));
        out.push_str(&format!("background = \"{}\"\n", hex(self.background)));
        out.push_str(&format!("cursor = \"{}\"\n", hex(self.cursor)));
        out.push_str(&format!("cursor_text = \"{}\"\n", hex(self.cursor_text)));
        if let Some(b) = self.bold { out.push_str(&format!("bold = \"{}\"\n", hex(b))); }
        if let Some(l) = self.link { out.push_str(&format!("link = \"{}\"\n", hex(l))); }
        if let Some(s) = self.selection { out.push_str(&format!("selection = \"{}\"\n", hex(s))); }
        out
    }

    /// Stage 13: deserialize from on-disk TOML.
    pub fn from_toml(s: &str) -> Result<Self, ThemeParseError> {
        #[derive(Deserialize)]
        struct File {
            name: String,
            ansi: AnsiMap,
            special: SpecialMap,
        }
        #[derive(Deserialize)]
        struct AnsiMap {
            ansi_0:  String, ansi_1:  String, ansi_2:  String, ansi_3:  String,
            ansi_4:  String, ansi_5:  String, ansi_6:  String, ansi_7:  String,
            ansi_8:  String, ansi_9:  String, ansi_10: String, ansi_11: String,
            ansi_12: String, ansi_13: String, ansi_14: String, ansi_15: String,
        }
        #[derive(Deserialize)]
        struct SpecialMap {
            foreground: String,
            background: String,
            cursor: String,
            cursor_text: String,
            #[serde(default)] bold: Option<String>,
            #[serde(default)] link: Option<String>,
            #[serde(default)] selection: Option<String>,
        }
        fn parse_hex(field: &str, s: &str) -> Result<[f32; 4], ThemeParseError> {
            let s = s.trim_start_matches('#');
            if s.len() != 6 {
                return Err(ThemeParseError::BadHex { field: field.into(), value: s.into() });
            }
            let r = u8::from_str_radix(&s[0..2], 16)
                .map_err(|_| ThemeParseError::BadHex { field: field.into(), value: s.into() })?;
            let g = u8::from_str_radix(&s[2..4], 16)
                .map_err(|_| ThemeParseError::BadHex { field: field.into(), value: s.into() })?;
            let b = u8::from_str_radix(&s[4..6], 16)
                .map_err(|_| ThemeParseError::BadHex { field: field.into(), value: s.into() })?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        let f: File = toml::from_str(s)?;
        let parse = |k: &str, v: &str| parse_hex(k, v);
        let ansi = [
            parse("ansi_0", &f.ansi.ansi_0)?, parse("ansi_1", &f.ansi.ansi_1)?,
            parse("ansi_2", &f.ansi.ansi_2)?, parse("ansi_3", &f.ansi.ansi_3)?,
            parse("ansi_4", &f.ansi.ansi_4)?, parse("ansi_5", &f.ansi.ansi_5)?,
            parse("ansi_6", &f.ansi.ansi_6)?, parse("ansi_7", &f.ansi.ansi_7)?,
            parse("ansi_8", &f.ansi.ansi_8)?, parse("ansi_9", &f.ansi.ansi_9)?,
            parse("ansi_10", &f.ansi.ansi_10)?, parse("ansi_11", &f.ansi.ansi_11)?,
            parse("ansi_12", &f.ansi.ansi_12)?, parse("ansi_13", &f.ansi.ansi_13)?,
            parse("ansi_14", &f.ansi.ansi_14)?, parse("ansi_15", &f.ansi.ansi_15)?,
        ];
        Ok(ThemeData {
            name: f.name,
            ansi,
            foreground: parse("foreground", &f.special.foreground)?,
            background: parse("background", &f.special.background)?,
            cursor: parse("cursor", &f.special.cursor)?,
            cursor_text: parse("cursor_text", &f.special.cursor_text)?,
            bold: f.special.bold.as_deref().map(|s| parse("bold", s)).transpose()?,
            link: f.special.link.as_deref().map(|s| parse("link", s)).transpose()?,
            selection: f.special.selection.as_deref().map(|s| parse("selection", s)).transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_theme() -> ThemeData {
        ThemeData {
            name: "test".into(),
            ansi: [[0.5, 0.5, 0.5, 1.0]; 16],
            foreground: [1.0, 1.0, 1.0, 1.0],
            background: [0.0, 0.0, 0.0, 1.0],
            cursor: [1.0, 0.0, 0.0, 1.0],
            cursor_text: [0.0, 0.0, 0.0, 1.0],
            bold: None,
            link: None,
            selection: None,
        }
    }

    #[test]
    fn to_toml_and_back_round_trips() {
        let t = sample_theme();
        let s = t.to_toml();
        let parsed = ThemeData::from_toml(&s).expect("roundtrip");
        assert_eq!(parsed.name, t.name);
        assert_eq!(parsed.foreground, t.foreground);
        assert_eq!(parsed.background, t.background);
        // Hex precision: 0.5 → 0x80 / 255.0 ≈ 0.5019. Approximate equality.
        for i in 0..16 {
            for c in 0..3 {
                let diff = (parsed.ansi[i][c] - t.ansi[i][c]).abs();
                assert!(diff < 0.01, "ansi[{i}][{c}] drift {diff}");
            }
        }
    }

    #[test]
    fn from_toml_rejects_invalid_hex() {
        let bad = r#"
name = "bad"
[ansi]
ansi_0 = "not-hex"
ansi_1 = "#000000"
ansi_2 = "#000000"
ansi_3 = "#000000"
ansi_4 = "#000000"
ansi_5 = "#000000"
ansi_6 = "#000000"
ansi_7 = "#000000"
ansi_8 = "#000000"
ansi_9 = "#000000"
ansi_10 = "#000000"
ansi_11 = "#000000"
ansi_12 = "#000000"
ansi_13 = "#000000"
ansi_14 = "#000000"
ansi_15 = "#000000"
[special]
foreground = "#ffffff"
background = "#000000"
cursor = "#000000"
cursor_text = "#000000"
"#;
        let r = ThemeData::from_toml(bad);
        assert!(matches!(r, Err(ThemeParseError::BadHex { .. })));
    }

    #[test]
    fn from_toml_rejects_missing_field() {
        let bad = r#"
name = "incomplete"
[ansi]
ansi_0 = "#000000"
"#;  // missing ansi_1..15 + special
        let r = ThemeData::from_toml(bad);
        assert!(r.is_err());
    }
}
```

Stage 13 adds two new deps: `thiserror = "1"` (for `ThemeParseError`) and `serde = { version = "1", features = ["derive"] }` (already a dep). Check `Cargo.toml`:

```bash
grep -n "thiserror\|^serde" crates/vibeflow/Cargo.toml
```

If `thiserror` isn't present, add it to `[dependencies]`. If you prefer to skip the dep, replace `#[derive(thiserror::Error)]` with a hand-rolled `impl std::error::Error` + `Display`.

- [ ] **Step 3: Empty stubs in iterm2.rs and registry.rs.**

`crates/vibeflow/src/theme/iterm2.rs`:
```rust
//! Stage 13: iTerm2 .itermcolors plist parser. Implementation in T11.
```

`crates/vibeflow/src/theme/registry.rs`:
```rust
//! Stage 13: theme registry — scans ~/.config/vibeflow/themes/. Implementation in T12.
```

These compile as empty modules so T10's `pub mod iterm2; pub mod registry;` in `mod.rs` works.

- [ ] **Step 4: Run tests + workspace.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --package vibeflow --lib theme 2>&1 | tail -10
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/theme/ crates/vibeflow/src/lib.rs crates/vibeflow/Cargo.toml
git commit -m "feat(stage13): theme module scaffold (ThemeData + TOML round-trip)"
```

---

### Task 11: iTerm2 plist parser (TDD)

**Files:**
- Modify: `crates/vibeflow/src/theme/iterm2.rs`
- Modify: `crates/vibeflow/Cargo.toml` (add `plist = "1"`)
- Create: `crates/vibeflow/tests/fixtures/sample.itermcolors`

- [ ] **Step 1: Add `plist` dep.**

```toml
# Cargo.toml [dependencies]
plist = "1"
```

Run `cargo build --workspace` to fetch.

- [ ] **Step 2: Create fixture `crates/vibeflow/tests/fixtures/sample.itermcolors`.**

```bash
mkdir -p crates/vibeflow/tests/fixtures
```

Save the following (a minimal valid .itermcolors with all required keys):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Ansi 0 Color</key>
    <dict>
        <key>Red Component</key><real>0.0</real>
        <key>Green Component</key><real>0.0</real>
        <key>Blue Component</key><real>0.0</real>
        <key>Alpha Component</key><real>1.0</real>
    </dict>
    <key>Ansi 1 Color</key>
    <dict>
        <key>Red Component</key><real>0.86274509803921573</real>
        <key>Green Component</key><real>0.19607843137254902</real>
        <key>Blue Component</key><real>0.18431372549019606</real>
    </dict>
    <key>Ansi 2 Color</key>
    <dict>
        <key>Red Component</key><real>0.52156862745098043</real>
        <key>Green Component</key><real>0.59999999999999998</real>
        <key>Blue Component</key><real>0.0</real>
    </dict>
    <key>Ansi 3 Color</key>
    <dict>
        <key>Red Component</key><real>0.70980392156862748</real>
        <key>Green Component</key><real>0.53725490196078429</real>
        <key>Blue Component</key><real>0.0</real>
    </dict>
    <key>Ansi 4 Color</key>
    <dict>
        <key>Red Component</key><real>0.14901960784313725</real>
        <key>Green Component</key><real>0.54509803921568623</real>
        <key>Blue Component</key><real>0.82352941176470584</real>
    </dict>
    <key>Ansi 5 Color</key>
    <dict>
        <key>Red Component</key><real>0.82745098039215681</real>
        <key>Green Component</key><real>0.21176470588235294</real>
        <key>Blue Component</key><real>0.5098039215686274</real>
    </dict>
    <key>Ansi 6 Color</key>
    <dict>
        <key>Red Component</key><real>0.16470588235294117</real>
        <key>Green Component</key><real>0.63137254901960782</real>
        <key>Blue Component</key><real>0.59607843137254901</real>
    </dict>
    <key>Ansi 7 Color</key>
    <dict>
        <key>Red Component</key><real>0.93333333333333335</real>
        <key>Green Component</key><real>0.90980392156862744</real>
        <key>Blue Component</key><real>0.83529411764705885</real>
    </dict>
    <key>Ansi 8 Color</key>
    <dict>
        <key>Red Component</key><real>0.0</real>
        <key>Green Component</key><real>0.16862745098039217</real>
        <key>Blue Component</key><real>0.21176470588235294</real>
    </dict>
    <key>Ansi 9 Color</key>
    <dict>
        <key>Red Component</key><real>0.79607843137254897</real>
        <key>Green Component</key><real>0.29411764705882354</real>
        <key>Blue Component</key><real>0.086274509803921567</real>
    </dict>
    <key>Ansi 10 Color</key>
    <dict>
        <key>Red Component</key><real>0.34509803921568627</real>
        <key>Green Component</key><real>0.43137254901960786</real>
        <key>Blue Component</key><real>0.45882352941176469</real>
    </dict>
    <key>Ansi 11 Color</key>
    <dict>
        <key>Red Component</key><real>0.396078431372549</real>
        <key>Green Component</key><real>0.4823529411764706</real>
        <key>Blue Component</key><real>0.5137254901960784</real>
    </dict>
    <key>Ansi 12 Color</key>
    <dict>
        <key>Red Component</key><real>0.51372549019607838</real>
        <key>Green Component</key><real>0.58039215686274515</real>
        <key>Blue Component</key><real>0.58823529411764708</real>
    </dict>
    <key>Ansi 13 Color</key>
    <dict>
        <key>Red Component</key><real>0.42352941176470588</real>
        <key>Green Component</key><real>0.44313725490196076</real>
        <key>Blue Component</key><real>0.7686274509803922</real>
    </dict>
    <key>Ansi 14 Color</key>
    <dict>
        <key>Red Component</key><real>0.57647058823529407</real>
        <key>Green Component</key><real>0.63137254901960782</real>
        <key>Blue Component</key><real>0.63137254901960782</real>
    </dict>
    <key>Ansi 15 Color</key>
    <dict>
        <key>Red Component</key><real>0.99215686274509807</real>
        <key>Green Component</key><real>0.96470588235294119</real>
        <key>Blue Component</key><real>0.89019607843137254</real>
    </dict>
    <key>Background Color</key>
    <dict>
        <key>Red Component</key><real>0.0</real>
        <key>Green Component</key><real>0.16862745098039217</real>
        <key>Blue Component</key><real>0.21176470588235294</real>
    </dict>
    <key>Foreground Color</key>
    <dict>
        <key>Red Component</key><real>0.51372549019607838</real>
        <key>Green Component</key><real>0.58039215686274515</real>
        <key>Blue Component</key><real>0.58823529411764708</real>
    </dict>
    <key>Cursor Color</key>
    <dict>
        <key>Red Component</key><real>0.57647058823529407</real>
        <key>Green Component</key><real>0.63137254901960782</real>
        <key>Blue Component</key><real>0.63137254901960782</real>
    </dict>
    <key>Cursor Text Color</key>
    <dict>
        <key>Red Component</key><real>0.0</real>
        <key>Green Component</key><real>0.16862745098039217</real>
        <key>Blue Component</key><real>0.21176470588235294</real>
    </dict>
</dict>
</plist>
```

This is a minimal Solarized Dark in iTerm2 format. The fixture is committed to the repo and used by `parse_itermcolors_round_trips_solarized_dark` test.

- [ ] **Step 3: Add tests in `theme/iterm2.rs::tests`.**

```rust
use super::*;
use crate::theme::ThemeData;

const SAMPLE: &[u8] = include_bytes!("../../tests/fixtures/sample.itermcolors");

#[test]
fn parse_itermcolors_round_trips_solarized_dark() {
    let t = parse_itermcolors(SAMPLE).expect("parse");
    // Spot-check known values from the fixture.
    // ansi_0 = (0, 0, 0)
    assert!((t.ansi[0][0] - 0.0).abs() < 0.01);
    assert!((t.ansi[0][1] - 0.0).abs() < 0.01);
    assert!((t.ansi[0][2] - 0.0).abs() < 0.01);
    // ansi_1 (red) ≈ (0.86, 0.19, 0.18)
    assert!((t.ansi[1][0] - 0.86).abs() < 0.02);
    assert!((t.ansi[1][1] - 0.19).abs() < 0.02);
    // Foreground ≈ (0.51, 0.58, 0.58)
    assert!((t.foreground[0] - 0.51).abs() < 0.02);
}

#[test]
fn parse_itermcolors_rejects_not_a_plist() {
    let r = parse_itermcolors(b"this is not a plist");
    assert!(matches!(r, Err(ItermImportError::NotAPlist(_))));
}

#[test]
fn parse_itermcolors_rejects_missing_required_key() {
    // Plist missing "Ansi 0 Color".
    let xml = br#"<?xml version="1.0"?>
<plist version="1.0"><dict>
<key>Background Color</key><dict><key>Red Component</key><real>0.0</real></dict>
</dict></plist>"#;
    let r = parse_itermcolors(xml);
    assert!(matches!(r, Err(ItermImportError::MissingKey(_))));
}
```

- [ ] **Step 4: Add `iterm2.rs` body.**

Replace the stub with:

```rust
//! Stage 13: iTerm2 .itermcolors plist parser.

use crate::theme::ThemeData;
use plist::Value;

#[derive(Debug, thiserror::Error)]
pub enum ItermImportError {
    #[error("not a valid plist: {0}")]
    NotAPlist(String),
    #[error("plist is not a dictionary")]
    NotADict,
    #[error("missing required key: {0}")]
    MissingKey(String),
    #[error("invalid color value for key {0}")]
    BadColorValue(String),
}

pub fn parse_itermcolors(plist_bytes: &[u8]) -> Result<ThemeData, ItermImportError> {
    let value: Value = plist::from_bytes(plist_bytes)
        .map_err(|e| ItermImportError::NotAPlist(e.to_string()))?;
    let dict = value.into_dictionary().ok_or(ItermImportError::NotADict)?;

    fn read_color(
        dict: &plist::Dictionary,
        key: &str,
    ) -> Result<[f32; 4], ItermImportError> {
        let sub = dict
            .get(key)
            .ok_or_else(|| ItermImportError::MissingKey(key.to_owned()))?
            .as_dictionary()
            .ok_or_else(|| ItermImportError::BadColorValue(key.to_owned()))?;
        let r = sub.get("Red Component").and_then(|v| v.as_real()).unwrap_or(0.0) as f32;
        let g = sub.get("Green Component").and_then(|v| v.as_real()).unwrap_or(0.0) as f32;
        let b = sub.get("Blue Component").and_then(|v| v.as_real()).unwrap_or(0.0) as f32;
        let a = sub.get("Alpha Component").and_then(|v| v.as_real()).unwrap_or(1.0) as f32;
        Ok([r, g, b, a])
    }

    let mut ansi = [[0.0_f32; 4]; 16];
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
        ansi,
        foreground,
        background,
        cursor,
        cursor_text,
        bold,
        link,
        selection,
    })
}

#[cfg(test)]
mod tests {
    // Tests defined above in Step 3 land here.
}
```

(Move the Step 3 test code into this `mod tests` block.)

- [ ] **Step 5: Run tests; expect pass.**

```bash
cargo test --package vibeflow --lib theme::iterm2 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 6: Quality gate + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/theme/iterm2.rs crates/vibeflow/tests/fixtures/sample.itermcolors crates/vibeflow/Cargo.toml
git commit -m "feat(stage13): iTerm2 plist parser + Solarized Dark fixture"
```

---

### Task 12: Theme registry (TDD)

**Files:**
- Modify: `crates/vibeflow/src/theme/registry.rs`

- [ ] **Step 1: Add `ThemeRegistry` with tests.**

Replace the stub with:

```rust
//! Stage 13: theme registry — scans `~/.config/vibeflow/themes/`.

use crate::theme::ThemeData;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ThemeRegistry {
    themes: HashMap<String, ThemeData>,
    themes_dir: PathBuf,
}

impl ThemeRegistry {
    pub fn new_empty() -> Self {
        Self { themes: HashMap::new(), themes_dir: PathBuf::new() }
    }

    /// Scan `themes_dir` for `*.toml` files. Invalid entries logged at warn.
    pub fn load(themes_dir: PathBuf) -> Self {
        let mut themes = HashMap::new();
        let Ok(entries) = std::fs::read_dir(&themes_dir) else {
            tracing::debug!(
                "themes dir not found at {}; registry will be empty",
                themes_dir.display()
            );
            return Self { themes, themes_dir };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let contents = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("cannot read theme {}: {e}", path.display());
                    continue;
                }
            };
            let theme = match ThemeData::from_toml(&contents) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("cannot parse theme {}: {e}", path.display());
                    continue;
                }
            };
            themes.insert(theme.name.clone(), theme);
        }
        Self { themes, themes_dir }
    }

    pub fn get(&self, name: &str) -> Option<&ThemeData> {
        self.themes.get(name)
    }

    /// Sorted theme names — for menu listing.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.themes.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn reload(&mut self) {
        *self = Self::load(self.themes_dir.clone());
    }

    pub fn themes_dir(&self) -> &std::path::Path {
        &self.themes_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_valid_theme(dir: &std::path::Path, name: &str) {
        let data = ThemeData {
            name: name.into(),
            ansi: [[0.5; 4]; 16],
            foreground: [1.0; 4],
            background: [0.0; 4],
            cursor: [0.5, 0.5, 0.5, 1.0],
            cursor_text: [0.0; 4],
            bold: None,
            link: None,
            selection: None,
        };
        std::fs::write(dir.join(format!("{name}.toml")), data.to_toml()).expect("write");
    }

    #[test]
    fn load_empty_dir_returns_empty_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert!(reg.names().is_empty());
    }

    #[test]
    fn load_picks_up_valid_themes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_theme(tmp.path(), "alpha");
        write_valid_theme(tmp.path(), "beta");
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert_eq!(reg.names(), vec!["alpha".to_string(), "beta".to_string()]);
        assert!(reg.get("alpha").is_some());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn load_skips_malformed_themes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_theme(tmp.path(), "good");
        std::fs::write(tmp.path().join("bad.toml"), "this is not valid toml }}}}").unwrap();
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert_eq!(reg.names(), vec!["good".to_string()]);
    }

    #[test]
    fn load_ignores_non_toml_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_theme(tmp.path(), "good");
        std::fs::write(tmp.path().join("README.md"), "not a theme").unwrap();
        std::fs::write(tmp.path().join("random.txt"), "also not a theme").unwrap();
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert_eq!(reg.names(), vec!["good".to_string()]);
    }
}
```

- [ ] **Step 2: Run tests.**

```bash
cargo test --package vibeflow --lib theme::registry 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 3: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/theme/registry.rs
git commit -m "feat(stage13): ThemeRegistry — scan ~/.config/vibeflow/themes/*.toml"
```

---

### Task 13: `--import-colors` CLI subcommand

**Files:**
- Modify: `crates/vibeflow/src/main.rs`
- Modify: `crates/vibeflow/Cargo.toml` (add `dirs = "5"` if not present)

- [ ] **Step 1: Add `dirs` dep if not present.**

```bash
grep -n "^dirs" crates/vibeflow/Cargo.toml
```

If not found, add to `[dependencies]`:
```toml
dirs = "5"
```

- [ ] **Step 2: Modify `main.rs` to intercept `--import-colors` before launching GUI.**

Read existing main:
```bash
sed -n '1,50p' crates/vibeflow/src/main.rs
```

Wrap the existing main with an arg-parsing prelude:

```rust
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--import-colors") {
        let Some(path_str) = args.get(pos + 1) else {
            eprintln!("usage: vibeflow --import-colors <path> [--overwrite]");
            return std::process::ExitCode::from(2);
        };
        let overwrite = args.iter().any(|a| a == "--overwrite");
        return run_import_colors(path_str, overwrite);
    }
    // ... existing main body — initialize tracing, run event loop.
    // Wrap whatever existed before in a helper or inline here.
    run_gui()
}

fn run_gui() -> std::process::ExitCode {
    // ...existing main body verbatim...
    std::process::ExitCode::SUCCESS
}

fn run_import_colors(path_str: &str, overwrite: bool) -> std::process::ExitCode {
    use std::path::Path;
    let in_path = Path::new(path_str);
    let bytes = match std::fs::read(in_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path_str}: {e}");
            return std::process::ExitCode::from(1);
        }
    };
    let mut theme = match vibeflow::theme::iterm2::parse_itermcolors(&bytes) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("parse error in {path_str}: {e}");
            return std::process::ExitCode::from(1);
        }
    };
    let name = in_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    if name == "default" {
        eprintln!("'default' is a reserved theme name");
        return std::process::ExitCode::from(1);
    }
    theme.name = name.to_owned();

    let themes_dir = dirs::config_dir().unwrap_or_default().join("vibeflow/themes");
    if let Err(e) = std::fs::create_dir_all(&themes_dir) {
        eprintln!("cannot create {}: {e}", themes_dir.display());
        return std::process::ExitCode::from(1);
    }
    let out_path = themes_dir.join(format!("{name}.toml"));
    if out_path.exists() && !overwrite {
        eprintln!(
            "theme '{name}' already exists at {}; use --overwrite to replace",
            out_path.display()
        );
        return std::process::ExitCode::from(1);
    }
    let toml_str = theme.to_toml();
    if let Err(e) = std::fs::write(&out_path, toml_str) {
        eprintln!("cannot write {}: {e}", out_path.display());
        return std::process::ExitCode::from(1);
    }
    println!("imported theme '{name}' to {}", out_path.display());
    std::process::ExitCode::SUCCESS
}
```

Adapt to the actual existing `main()` shape — the existing body's setup logic (tracing init, app construction, event loop) goes into `run_gui()`.

- [ ] **Step 3: Smoke test the CLI manually.**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --release --bin vibeflow 2>&1 | tail -3
./target/release/vibeflow --import-colors crates/vibeflow/tests/fixtures/sample.itermcolors --overwrite
ls -la ~/.config/vibeflow/themes/sample.toml
cat ~/.config/vibeflow/themes/sample.toml | head -20
```

Expected: success message, file created, contents look like a valid theme TOML.

- [ ] **Step 4: Quality gate + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/main.rs crates/vibeflow/Cargo.toml
git commit -m "feat(stage13): --import-colors CLI subcommand"
```

---

### Task 14: PtySession.theme + set_theme + restart preservation (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`

- [ ] **Step 1: Add `theme: Option<String>` field on PtySession and constructor init.**

```rust
    /// Stage 13: theme name (None = use Stage 9 hardcoded defaults).
    /// Mirror of `Config.colors.preset`; can be overridden per-tab via
    /// the Stage 10 context menu.
    pub(crate) theme: Option<String>,
```

Initialize in `spawn`'s `Ok(Self { ... })`:
```rust
            theme: None,
```

In `restart()`'s body, preserve theme — same pattern as `history_lines` does:
```rust
        let theme = self.theme.clone();
        // (existing rebuild via Self::spawn)
        *self = new_session;
        self.theme = theme;
        // T15's App::restart_active will call set_theme to actually apply.
```

- [ ] **Step 2: Add `apply_theme_to_colors` private helper + `set_theme` public method.**

In `session.rs`:

```rust
fn apply_theme_to_colors(
    colors: &mut alacritty_terminal::term::color::Colors,
    theme: &crate::theme::ThemeData,
) {
    use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
    fn to_rgb(c: [f32; 4]) -> Rgb {
        Rgb {
            r: (c[0].clamp(0.0, 1.0) * 255.0) as u8,
            g: (c[1].clamp(0.0, 1.0) * 255.0) as u8,
            b: (c[2].clamp(0.0, 1.0) * 255.0) as u8,
        }
    }
    let named_for_ansi = |i: usize| -> NamedColor {
        match i {
            0 => NamedColor::Black, 1 => NamedColor::Red,
            2 => NamedColor::Green, 3 => NamedColor::Yellow,
            4 => NamedColor::Blue, 5 => NamedColor::Magenta,
            6 => NamedColor::Cyan, 7 => NamedColor::White,
            8 => NamedColor::BrightBlack, 9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen, 11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue, 13 => NamedColor::BrightMagenta,
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
    if let Some(b) = theme.bold {
        colors[NamedColor::Bold] = Some(to_rgb(b));
    }
    // link / selection don't map to NamedColor directly — handled by render layer in future stages.
}

impl PtySession {
    /// Stage 13: apply a named theme to this session, or restore Stage 9
    /// defaults when `name` is None. If the named theme isn't in the
    /// registry, logs warn and keeps current colors.
    pub fn set_theme(
        &mut self,
        name: Option<String>,
        registry: &crate::theme::registry::ThemeRegistry,
    ) {
        self.theme = name.clone();
        let Some(theme_name) = name else {
            // Restore defaults.
            *self.term.colors_mut() = alacritty_terminal::term::color::Colors::default();
            return;
        };
        let Some(theme) = registry.get(&theme_name) else {
            tracing::warn!("theme '{theme_name}' not found in registry; keeping current colors");
            return;
        };
        apply_theme_to_colors(self.term.colors_mut(), theme);
    }
}
```

**Verify `Colors[NamedColor]` indexable** — alacritty_terminal's `Colors` impl `Index<NamedColor>`. Read `alacritty_terminal-0.24.2/src/term/color.rs` to confirm during implementation. If the API differs (e.g., needs `colors.set(NamedColor::Foreground, ...)` instead of index), adapt. The senior pre-execution review catches drift.

- [ ] **Step 3: Add test.**

```rust
    #[test]
    fn set_theme_with_none_resets_to_default() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let registry = crate::theme::registry::ThemeRegistry::new_empty();
        s.set_theme(None, &registry);
        assert_eq!(s.theme, None);
    }

    #[test]
    fn set_theme_with_missing_name_keeps_current() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let registry = crate::theme::registry::ThemeRegistry::new_empty();
        s.set_theme(Some("nonexistent".to_owned()), &registry);
        // self.theme is still set to the requested name even if not found.
        assert_eq!(s.theme, Some("nonexistent".to_owned()));
    }
```

- [ ] **Step 4: Run tests + workspace.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --package vibeflow --lib session 2>&1 | tail -10
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```

- [ ] **Step 5: Commit.**

```bash
git add crates/vibeflow/src/session/session.rs
git commit -m "feat(stage13): PtySession.theme + set_theme + restart preserves"
```

---

### Task 15: App default_theme + propagation (TDD)

**Files:**
- Modify: `crates/vibeflow/src/app.rs`

- [ ] **Step 1: Add field + setter.**

```rust
    /// Stage 13: mirror of `Config.colors.preset`. Applied to subsequently-spawned
    /// tabs AND on restart.
    default_theme: Option<String>,
```

In `App::new()`:
```rust
            default_theme: None,
```

In `impl App`:
```rust
    /// Stage 13: update the default theme name for subsequently-spawned tabs.
    pub fn set_default_theme(&mut self, name: Option<String>) {
        self.default_theme = name;
    }
```

- [ ] **Step 2: Propagate in `new_tab` and `restart_active`.**

`App::new_tab`:
```rust
        session.theme = self.default_theme.clone();
        // The actual color write happens via apply_config or context menu — T17.
        self.tabs.push(session);
```

`App::restart_active`:
```rust
        s.theme = self.default_theme.clone();
        s.restart()?;
        // Re-apply after restart since term.colors_mut() resets on the new Term.
        // T16 wires the actual set_theme call from WindowApp.
```

Actually we need to call `set_theme(&registry)` to apply colors. App doesn't own the registry; WindowApp does. So App stores the name only; the actual color application happens in WindowApp::apply_config (T17) or WindowApp's restart handler.

Re-shape: `App.default_theme` is the NAME; PtySession.theme is also the name. The actual color application is in WindowApp::apply_config (which has access to `self.theme_registry`). On restart, WindowApp's handler for restart-completed re-calls `s.set_theme(s.theme.clone(), &registry)` to re-apply.

For Stage 13, the cleanest path is:

After `App::restart_active` returns to WindowApp's caller (the `Shortcut::RestartTab` handler in `handle_shortcut`), WindowApp re-applies the theme:

```rust
// In window.rs handle_shortcut for RestartTab:
Shortcut::RestartTab => {
    if let Err(e) = self.app.restart_active() {
        tracing::warn!("restart failed: {e}");
    }
    // Stage 13: re-apply theme to the freshly-restarted session.
    let active = self.app.active();
    let theme_name = self.app.tabs()[active].theme.clone();
    if let Some(s) = self.app.tabs_mut().get_mut(active) {
        s.set_theme(theme_name, &self.theme_registry);
    }
}
```

(T16 will set up `self.theme_registry`.)

- [ ] **Step 3: Add test.**

```rust
    #[test]
    fn new_tab_inherits_default_theme() {
        let mut app = App::new();
        app.set_default_theme(Some("solarized".to_owned()));
        let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        assert_eq!(app.tabs()[0].theme.as_deref(), Some("solarized"));
    }
```

- [ ] **Step 4: Quality gate + commit.**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/app.rs
git commit -m "feat(stage13): App.default_theme + setter + new_tab/restart propagation"
```

---

### Task 16: Context menu Theme list + `MenuAction::SetTheme` dispatch

**Files:**
- Modify: `crates/vibeflow/src/render/context_menu.rs`
- Modify: `crates/vibeflow/src/window.rs` (theme_registry field + dispatch)

- [ ] **Step 1: Add `MenuAction::SetTheme(String)` variant.**

In `render/context_menu.rs`'s `MenuAction` enum (Stage 10 added it):
```rust
    /// Stage 13: apply a named theme to the target tab.
    SetTheme(String),
```

- [ ] **Step 2: Extend `tab_menu` to optionally append theme items.**

Current signature (Stage 10):
```rust
pub fn tab_menu(target_idx: SessionIdx, is_dead: bool, tab_count: usize) -> Vec<MenuItem>
```

New signature accepting theme names:
```rust
pub fn tab_menu(
    target_idx: SessionIdx,
    is_dead: bool,
    tab_count: usize,
    theme_names: &[String],
) -> Vec<MenuItem>
```

In the body, after the existing menu items, append (if `!theme_names.is_empty()`):
```rust
    items.push(MenuItem::separator());
    for name in theme_names {
        items.push(MenuItem {
            label_owned: Some(format!("Theme: {}", name)),
            label: "",
            shortcut_hint: None,
            action: MenuAction::SetTheme(name.clone()),
            enabled: true,
            kind: ItemKind::Action,
        });
    }
```

The existing `MenuItem.label: &'static str` doesn't accept owned strings. Add a `label_owned: Option<String>` field to MenuItem (precedence: if Some, use it; else fall back to `label`). Update the renderer (`build_glyphs` in `render::context_menu` or wherever label-text is emitted) to check `label_owned` first.

Alternative simpler path: make `MenuItem.label` itself a `String` instead of `&'static str`. Migrate all existing static labels to `String` via `.into()`. This is a wider refactor but cleaner. Recommend this path; the cost of `.into()` in Stage 10's builders is trivial.

Implementer's choice — read `render/context_menu.rs` and decide based on which feels less invasive. The senior pre-exec review will catch if the implementer picks an inconsistent path.

- [ ] **Step 3: Update existing tab_menu test fixtures + callers.**

```bash
grep -rn "tab_menu(" crates/vibeflow/src/ crates/vibeflow/tests/
```

Most callers will pass `&[]` for `theme_names`. The actual usage (in `window.rs::open_context_menu`) passes `self.theme_registry.names().as_slice()`.

- [ ] **Step 4: Add `theme_registry: ThemeRegistry` field on WindowApp.**

```rust
    /// Stage 13: theme registry loaded at startup, refreshed on config reload.
    theme_registry: crate::theme::registry::ThemeRegistry,
```

In `WindowApp::new`:
```rust
            theme_registry: {
                let themes_dir = dirs::config_dir()
                    .unwrap_or_default()
                    .join("vibeflow/themes");
                crate::theme::registry::ThemeRegistry::load(themes_dir)
            },
```

- [ ] **Step 5: Update `open_context_menu` to pass theme names to `tab_menu`.**

Find the `tab_menu(...)` call:
```bash
grep -n "tab_menu(" crates/vibeflow/src/window.rs
```

Update:
```rust
let names = self.theme_registry.names();
let items = context_menu::tab_menu(idx, is_dead, tab_count, &names);
```

- [ ] **Step 6: Dispatch `MenuAction::SetTheme` in `dispatch_menu_action`.**

In `window.rs::dispatch_menu_action`'s match (Stage 10 / Stage 12):
```rust
    MenuAction::SetTheme(name) => {
        let target = target_idx.unwrap_or_else(|| self.app.active());
        if let Some(s) = self.app.tabs_mut().get_mut(target) {
            s.set_theme(Some(name), &self.theme_registry);
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
```

- [ ] **Step 7: Quality gate + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/render/context_menu.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage13): context menu Theme list + SetTheme dispatch"
```

---

### Task 17: `apply_config` wires `[colors] preset` + `[bell]` + `[scrollback] snap_on_esc`

**Files:**
- Modify: `crates/vibeflow/src/config/schema.rs` (`ColorsSection.preset`)
- Modify: `crates/vibeflow/src/config/mod.rs` (`Colors.preset` + `apply_colors`)
- Modify: `crates/vibeflow/src/window.rs::apply_config`

- [ ] **Step 1: Add `preset: Option<String>` to `ColorsSection` and `Colors`.**

In `config/schema.rs::ColorsSection`:
```rust
    pub preset: Option<String>,
```

In `config/mod.rs::Colors`:
```rust
    pub preset: Option<String>,
```

In `Config::default_values()`'s `colors:` literal:
```rust
                preset: None,
```

In `apply_colors`:
```rust
    if let Some(p) = section.preset {
        resolved.preset = Some(p);
    }
```

- [ ] **Step 2: Extend `WindowApp::apply_config`.**

In the existing `apply_config` body (find via `grep -n "fn apply_config" crates/vibeflow/src/window.rs`), at the end of the existing `[scrollback]` block, add:

```rust
        // Stage 13: snap_on_esc cache.
        self.snap_on_esc = sb.snap_on_esc;
```

Add a new block for `[bell]`:
```rust
        // Stage 13: [bell] section.
        let bell = &config.bell;
        self.bell_mode = bell.mode;
        self.bell_debounce = std::time::Duration::from_millis(bell.debounce_ms);
```

Add a new block for `[colors] preset` + theme reload:
```rust
        // Stage 13: theme preset.
        let new_preset = config.colors.preset.clone();
        self.app.set_default_theme(new_preset.clone());
        self.theme_registry.reload();
        for s in self.app.tabs_mut().iter_mut() {
            s.set_theme(new_preset.clone(), &self.theme_registry);
        }
```

- [ ] **Step 3: Quality gate + commit.**

```bash
cargo fmt --all
cargo build --workspace 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
git add crates/vibeflow/src/config/schema.rs crates/vibeflow/src/config/mod.rs crates/vibeflow/src/window.rs
git commit -m "feat(stage13): apply_config wires [colors] preset + [bell] + snap_on_esc"
```

---

### Task 18: Integration tests for themes

**Files:**
- Create: `crates/vibeflow/tests/themes.rs`

- [ ] **Step 1: Create the test file.**

```rust
//! Stage 13 integration tests — theme registry + per-session application
//! against a real PTY and real filesystem.

use std::time::{Duration, Instant};
use vibeflow::app::App;
use vibeflow::theme::registry::ThemeRegistry;
use vibeflow::theme::ThemeData;

fn drive_until(app: &mut App, deadline: Instant) {
    while Instant::now() < deadline {
        let now = Instant::now();
        let _ = app.poll_all(now);
        let _ = app.tick_all(now);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn make_theme(name: &str, fg: [f32; 4], bg: [f32; 4]) -> ThemeData {
    ThemeData {
        name: name.into(),
        ansi: [[0.5; 4]; 16],
        foreground: fg,
        background: bg,
        cursor: [0.5; 4],
        cursor_text: [0.0; 4],
        bold: None,
        link: None,
        selection: None,
    }
}

#[test]
fn theme_set_applies_to_term_colors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    let theme = make_theme("test_theme", red, blue);
    std::fs::write(
        tmp.path().join("test_theme.toml"),
        theme.to_toml(),
    ).expect("write");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf());
    assert!(registry.get("test_theme").is_some());

    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(200));
    let active = app.active();
    app.tabs_mut()[active].set_theme(Some("test_theme".into()), &registry);
    // Verify Term's color slot was updated.
    use alacritty_terminal::vte::ansi::NamedColor;
    let fg_color = app.tabs()[active].term().colors()[NamedColor::Foreground];
    assert!(fg_color.is_some(), "foreground should be set after apply");
    let rgb = fg_color.unwrap();
    // 1.0 * 255 = 255.
    assert_eq!(rgb.r, 255);
    assert_eq!(rgb.g, 0);
    assert_eq!(rgb.b, 0);
}

#[test]
fn theme_per_tab_isolation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = make_theme("alpha", [1.0, 0.0, 0.0, 1.0], [0.0; 4]);
    let b = make_theme("beta",  [0.0, 1.0, 0.0, 1.0], [0.0; 4]);
    std::fs::write(tmp.path().join("alpha.toml"), a.to_toml()).expect("write");
    std::fs::write(tmp.path().join("beta.toml"), b.to_toml()).expect("write");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf());

    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn");
    let _ = app.new_tab(&["bash"]).expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(300));

    app.tabs_mut()[0].set_theme(Some("alpha".into()), &registry);
    app.tabs_mut()[1].set_theme(Some("beta".into()), &registry);

    use alacritty_terminal::vte::ansi::NamedColor;
    let t0_fg = app.tabs()[0].term().colors()[NamedColor::Foreground].unwrap();
    let t1_fg = app.tabs()[1].term().colors()[NamedColor::Foreground].unwrap();
    assert_eq!(t0_fg.r, 255);
    assert_eq!(t1_fg.g, 255);
}

#[test]
fn missing_theme_logs_warn_keeps_current() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf());  // empty registry

    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(200));
    let active = app.active();
    app.tabs_mut()[active].set_theme(Some("ghost".into()), &registry);
    // self.theme is still set even if registry doesn't have the name.
    assert_eq!(app.tabs()[active].theme.as_deref(), Some("ghost"));
}
```

- [ ] **Step 2: Run.**

```bash
cargo test --package vibeflow --tests themes 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 3: Quality gate (full).**

```bash
cargo test --workspace 2>&1 | tail -10
cargo fmt --all
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
cargo build --release 2>&1 | tail -3
```

- [ ] **Step 4: Commit.**

```bash
git add crates/vibeflow/tests/themes.rs
git commit -m "test(stage13): integration tests for theme apply + per-tab isolation"
```

---

## Manual smoke walk (after Task 18 passes)

Walk the spec's 15-item manual smoke walk on slmbeast VNC:

```bash
cargo build --release
pkill -f 'target/release/vibeflow' 2>/dev/null
DISPLAY=:1 RUST_LOG=vibeflow=info ./target/release/vibeflow &
```

Items per spec section "Manual smoke walk on VNC" (1–15). Fix anything surfaced; each fix gets its own conventional-commit message.

## Senior holistic review

After smoke walk passes, dispatch a Sonnet-tier holistic review. Reviewer prompt sketch:

> Read the Stage 13 plan, spec, and every commit on this branch. Identify (a) design-level mistakes that span files and (b) cross-task consistency drift. Specifically: does `apply_config` correctly call `theme_registry.reload()` BEFORE `set_theme`? Do all `tab_menu(...)` callers pass the theme names slice? Do `Renderer` and `WindowApp` share the same view of `bell_mode` after hot-reload? Does `restart_active` re-apply theme via WindowApp (since App doesn't have registry access)? Report Critical / Important / Minor.

Apply Critical fixes; apply Important unless cost is high; note Minor.

## Plan self-review checklist

Spec coverage:
- [x] Indicator prominence (T1)
- [x] Bell config schema + dispatch + audible helper (T3 + T4)
- [x] iTerm2 theme system: scaffold (T10) + parser (T11) + registry (T12) + CLI (T13) + per-tab application (T14) + App propagation (T15) + context menu (T16) + apply_config wiring (T17)
- [x] Shift/Ctrl arrow keys (T5)
- [x] Block selection (T7)
- [x] Shift-extend selection anchor (T6)
- [x] Esc snap config knob (T2)
- [x] Font priority live-reload (T9)
- [x] Integration tests (T18)
- [x] Manual smoke walk (post-T18)

Cross-task type consistency:
- `ThemeData` defined in T10; consumed by T11 (parser output), T12 (registry value), T14 (set_theme input via `&ThemeData` from registry.get), T18 (test fixtures).
- `ThemeRegistry::load(themes_dir)`, `get(&str)`, `names() -> Vec<String>`, `reload()` — names consistent across T12 def, T16 caller, T17 caller, T18 test.
- `MenuAction::SetTheme(String)` defined T16; dispatched T16.
- `SelectionTracker::mouse_down(point, shift, alt, term, now)` signature change in T6 (5 args); all callers updated in T6; alt actually consumed in T7; window.rs call updated in T8.
- `Config.colors.preset: Option<String>` added T17; read in T17's apply_config wiring; matches schema field.
- `BellMode` enum + `Bell` resolved struct added T3; used in T4 dispatch + T17 apply_config wiring.

No placeholder text found except for "Implementation TBD" in the spec's risks section (refers to fontdb 0.16 API choice — implementer picks at T9 implementation time after reading the crate; legitimate to defer).
