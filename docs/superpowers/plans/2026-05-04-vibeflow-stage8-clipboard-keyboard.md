# vibeflow Stage 8 Implementation Plan: Clipboard + Keyboard Shortcuts + Selection

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add keyboard shortcuts (`Ctrl+Shift+{T,W,Tab,R,C,V}` plus `Super+...` aliases), mouse selection (drag, double-click word, triple-click line), system clipboard integration with bracketed paste, dead-session restart, and terminal mouse-mode passthrough.

**Architecture:** Three pure-logic modules (`keymap`, `mouse_encoder`, `render/selection`) plus a thin `clipboard` wrapper over the `arboard` crate. The `SelectionTracker` lives on each `PtySession`, so selection state is per-tab and persists across tab switches. `window.rs`'s existing dispatch is extended with a `match_shortcut` call before the typed-input fallthrough, and mouse events are routed to either the tracker or the PTY (as SGR escape sequences) based on the term's mouse-mode flag, with `Shift` as a per-event override.

**Tech Stack:** New dependency: `arboard = "3.6"` (system clipboard, X11 + Wayland + macOS + Windows). Existing: `winit 0.30` (keyboard/mouse events, ModifiersState), `alacritty_terminal 0.24.2` (TermMode flags, semantic_search_left/right, Point indexing), `wgpu 0.20` (no GPU-side changes — we reuse the post-Stage-7.5 `tab_bar_pipeline` and `quad_pipeline` with their `write_uniform_and_instances` + `draw_range` API).

**Lessons carried forward from Stages 1–7.5:**
- Pre-execution senior review of plan code is high-value. Run a Sonnet review pass on this plan before dispatching tasks.
- Per-task Haiku reviewers consistently miss whole-stage issues. Run a final senior-tier holistic review before merging.
- Implementers will sometimes use refactor tasks to rewrite UNRELATED tests with fabricated justifications. Compare test-name lists before/after every multi-file refactor.
- Plan-verbatim Rust must be rustfmt-clean.
- Use `#[allow(clippy::too_many_arguments)]` for functions with > 7 parameters (precedent: `QuadInstance::new`, `build_cell_instances`).
- `pub fn` items don't trigger `dead_code` warnings. Don't reintroduce `#[allow(dead_code)]`.
- WGSL bugs surface only at runtime; smoke is the validation gate. Stage 8 doesn't change WGSL, so this is informational.
- VNC display is available on host (port 5901). GUI smoke runs are runnable.
- `cargo` invoked from outside `/path/to/vibeflow` may pick up another `Cargo.toml`. Always prefix `cd /path/to/vibeflow &&` or use absolute paths.

---

## File Structure

| Path | Responsibility | Net delta |
|---|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Add `arboard = "3.6"` dependency. | +1 / 0 |
| `crates/vibeflow/src/lib.rs` (modify) | Declare new modules: `pub mod clipboard;` `pub mod keymap;`. | +2 / 0 |
| `crates/vibeflow/src/render/mod.rs` (modify) | Declare new sub-modules: `pub mod mouse_encoder;` `pub mod selection;`. Add selection rect generation in `Renderer::render`. Add one new `tab_bar_pipeline.draw_range(...)` call. | +50 / -2 |
| `crates/vibeflow/src/clipboard.rs` (create) | `Clipboard` wrapper over `arboard::Clipboard`. CLIPBOARD-only. | +60 |
| `crates/vibeflow/src/keymap.rs` (create) | `Shortcut` enum + `match_shortcut` dispatch table for Ctrl+Shift+... and Super+... combos. | +120 |
| `crates/vibeflow/src/render/mouse_encoder.rs` (create) | SGR + X10 mouse-event encoding for terminal mouse-mode passthrough. | +130 |
| `crates/vibeflow/src/render/selection.rs` (create) | `SelectionTracker` state machine + `Selection` value type. Pure logic; no GPU/winit/PTY deps. | +250 |
| `crates/vibeflow/src/session/session.rs` (modify) | Add `selection: SelectionTracker` field. Add `pub fn restart(&mut self)`. | +50 / -0 |
| `crates/vibeflow/src/app.rs` (modify) | Add `restart_active(&mut self)` and `cycle_active(&mut self, dir: i32)`. | +30 / -0 |
| `crates/vibeflow/src/window.rs` (modify) | Keyboard shortcut dispatch in `KeyboardInput`. Mouse event routing (selection vs PTY mouse mode) in `MouseInput` + `CursorMoved`. Bracketed-paste wrapping in the paste handler. | +180 / -10 |
| `docs/TESTING.md` (modify) | Append Stage 8 manual smoke checklist. | +50 |

**Net add:** ~+925 / −12 (≈ +913 net), 5 files modified, 4 files created, 1 dep added.

---

## Task 0: Branch + `arboard` dependency + module scaffolding

**Files:**
- Create branch: `stage8-clipboard-keyboard` from `main` (commit `5f82e1d`).
- Modify: `crates/vibeflow/Cargo.toml`
- Modify: `crates/vibeflow/src/lib.rs`
- Modify: `crates/vibeflow/src/render/mod.rs`
- Create: `crates/vibeflow/src/clipboard.rs` (empty stub)
- Create: `crates/vibeflow/src/keymap.rs` (empty stub)
- Create: `crates/vibeflow/src/render/selection.rs` (empty stub)
- Create: `crates/vibeflow/src/render/mouse_encoder.rs` (empty stub)

This task adds the `arboard` dep and creates empty module stubs so later tasks have a place to grow. NO functionality yet.

- [ ] **Step 1: Create the branch**

```bash
cd /path/to/vibeflow
git checkout main
git pull --ff-only || true
git checkout -b stage8-clipboard-keyboard
```

- [ ] **Step 2: Add `arboard` dependency**

Open `crates/vibeflow/Cargo.toml`. Find the `[dependencies]` section. Add:

```toml
arboard = "3.6"
```

Place it alphabetically (after `anyhow`, before `bytemuck`). Example resulting block (relative ordering — don't reorder unrelated deps):

```toml
[dependencies]
alacritty_terminal = "0.24"
anyhow = "1.0"
arboard = "3.6"
bytemuck = { version = "1.16", features = ["derive"] }
```

- [ ] **Step 3: Create empty module stubs**

```bash
cd /path/to/vibeflow
touch crates/vibeflow/src/clipboard.rs
touch crates/vibeflow/src/keymap.rs
touch crates/vibeflow/src/render/selection.rs
touch crates/vibeflow/src/render/mouse_encoder.rs
```

Each file currently has zero bytes. Add a one-line module-level doc to each so `RUSTDOCFLAGS="-D warnings" cargo doc` doesn't complain about "missing docs":

`crates/vibeflow/src/clipboard.rs`:
```rust
//! System clipboard wrapper. Stage 8 uses CLIPBOARD only; PRIMARY is deferred.
```

`crates/vibeflow/src/keymap.rs`:
```rust
//! Keyboard shortcut dispatch. Single source of truth for the
//! Ctrl+Shift+... and Super+... shortcut table.
```

`crates/vibeflow/src/render/selection.rs`:
```rust
//! Mouse-driven cell selection state machine. Pure logic — no GPU, no winit,
//! no PTY dependency.
```

`crates/vibeflow/src/render/mouse_encoder.rs`:
```rust
//! Encodes mouse events into SGR (preferred) or X10 (legacy) escape
//! sequences for terminal mouse-mode passthrough into vim/htop/tmux/etc.
```

- [ ] **Step 4: Declare the new top-level modules in `lib.rs`**

Open `crates/vibeflow/src/lib.rs`. Find the existing `pub mod ...;` block. Add:

```rust
pub mod clipboard;
pub mod keymap;
```

Place alphabetically among the existing module declarations.

- [ ] **Step 5: Declare the new sub-modules in `render/mod.rs`**

Open `crates/vibeflow/src/render/mod.rs`. Find the existing `pub mod ...;` block at the top (likely contains `pub mod cursor; pub mod quad; pub mod tabs; pub mod text_engine;` or similar). Add:

```rust
pub mod mouse_encoder;
pub mod selection;
```

Place alphabetically.

- [ ] **Step 6: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean. 135 default tests pass + 12 ignored. fmt/clippy clean.

If you see "unresolved import `arboard`" or "no crate found", verify Cargo.toml edit is syntactically valid.

If you see warnings like `module ... is never used`, that's because the new modules are empty. Don't suppress with `#[allow(dead_code)]` — once Task 1+ adds public items, the warning disappears. If clippy *fails* on this, it's probably warning about "missing-docs" — verify the module-level doc comment was added.

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/Cargo.toml \
        crates/vibeflow/Cargo.lock \
        crates/vibeflow/src/lib.rs \
        crates/vibeflow/src/render/mod.rs \
        crates/vibeflow/src/clipboard.rs \
        crates/vibeflow/src/keymap.rs \
        crates/vibeflow/src/render/selection.rs \
        crates/vibeflow/src/render/mouse_encoder.rs
git commit -m "chore: scaffold Stage 8 modules + add arboard dep"
```

If `cargo build` modified `Cargo.lock`, include it. If not, omit. The `git add` of a non-existent path is silently ignored.

---

## Task 1: `keymap` — `Shortcut` enum + `match_shortcut` dispatch (TDD)

**Files:**
- Modify: `crates/vibeflow/src/keymap.rs`

`keymap` is pure logic — no GPU, no winit-specific imports beyond the type signatures. We TDD the full table.

- [ ] **Step 1: Write failing tests + Shortcut enum + match_shortcut signature**

Replace the contents of `crates/vibeflow/src/keymap.rs` with:

```rust
//! Keyboard shortcut dispatch. Single source of truth for the
//! Ctrl+Shift+... and Super+... shortcut table.
//!
//! `match_shortcut` consumes a winit logical key + modifier set and returns
//! `Some(Shortcut)` if the combo matches one of vibeflow's shortcuts;
//! otherwise `None`, in which case the caller should fall through to the
//! ordinary typed-input path (so a literal `T` keystroke still reaches the
//! PTY when no Ctrl+Shift / Super modifier is held).

use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Discrete shortcut actions vibeflow's `window.rs` dispatches. Stage 9 will
/// extend this enum with config-driven entries; for now it's hard-coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    RestartTab,
    Copy,
    Paste,
}

/// Match a winit key event against the shortcut table. Returns `None` if the
/// modifier combo doesn't match any shortcut — the caller should then fall
/// through to typed-input dispatch.
///
/// Matching rule:
/// * `Ctrl+Shift+Key` is the primary form.
/// * `Super+Key` (no Shift required, except for `Tab` directionals) is an
///   alias — gives Mac users via VNC the muscle-memory of `Cmd+...`.
/// * `Alt` and other non-listed modifiers must be UNSET. Pressing
///   `Ctrl+Shift+Alt+T` is NOT a new-tab shortcut — it falls through.
#[must_use]
pub fn match_shortcut(key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();
    let alt = modifiers.alt_key();
    let supr = modifiers.super_key();

    // Reject any combo with Alt set — we don't bind Alt-anything in Stage 8.
    if alt {
        return None;
    }

    // The two valid modifier shapes for the non-Tab shortcuts:
    //   * Ctrl+Shift  (standard Linux terminal binding)
    //   * Super alone (Mac-via-VNC alias)
    // Tab is the exception: prev-tab needs Shift either way.
    let ctrl_shift = ctrl && shift && !supr;
    let super_only = supr && !ctrl && !shift;
    let super_shift = supr && shift && !ctrl;

    match key {
        Key::Character(c) if ctrl_shift || super_only => match c.as_str() {
            "T" | "t" => Some(Shortcut::NewTab),
            "W" | "w" => Some(Shortcut::CloseTab),
            "R" | "r" => Some(Shortcut::RestartTab),
            "C" | "c" => Some(Shortcut::Copy),
            "V" | "v" => Some(Shortcut::Paste),
            _ => None,
        },
        Key::Named(NamedKey::Tab) => {
            // Ctrl+Tab → next; Ctrl+Shift+Tab → prev.
            // Super+Tab → next; Super+Shift+Tab → prev.
            if ctrl && !shift && !supr {
                Some(Shortcut::NextTab)
            } else if ctrl && shift && !supr {
                Some(Shortcut::PrevTab)
            } else if supr && !shift && !ctrl {
                Some(Shortcut::NextTab)
            } else if super_shift {
                Some(Shortcut::PrevTab)
            } else {
                None
            }
        }
        _ => None,
    }
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
        if ctrl { m |= ModifiersState::CONTROL; }
        if shift { m |= ModifiersState::SHIFT; }
        if alt { m |= ModifiersState::ALT; }
        if supr { m |= ModifiersState::SUPER; }
        m
    }

    // Ctrl+Shift form

    #[test]
    fn ctrl_shift_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, true, false, false)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn ctrl_shift_lowercase_t_is_new_tab() {
        // winit may deliver lowercase or uppercase depending on Shift state;
        // accept both so layout differences don't bite us.
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

    // Super-alias form

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

    // Negative cases

    #[test]
    fn plain_t_is_none() {
        assert_eq!(match_shortcut(&ch("T"), mods(false, false, false, false)), None);
    }

    #[test]
    fn ctrl_t_without_shift_is_none() {
        // Ctrl+T alone is not a vibeflow shortcut; bash uses it for transpose-chars.
        assert_eq!(match_shortcut(&ch("T"), mods(true, false, false, false)), None);
    }

    #[test]
    fn ctrl_shift_alt_t_is_none() {
        // Alt set → reject. Don't false-positive on Ctrl+Shift+Alt+T.
        assert_eq!(match_shortcut(&ch("T"), mods(true, true, true, false)), None);
    }

    #[test]
    fn ctrl_shift_x_is_none() {
        // Unbound character with the right modifiers still returns None.
        assert_eq!(match_shortcut(&ch("X"), mods(true, true, false, false)), None);
    }

    #[test]
    fn super_with_ctrl_is_none() {
        // Super+Ctrl combo is ambiguous; reject to keep the table boring.
        assert_eq!(match_shortcut(&ch("T"), mods(true, false, false, true)), None);
    }
}
```

- [ ] **Step 2: Verify the tests pass and existing tests don't change**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib keymap 2>&1 | tail -10
```

Expected: 17 new tests pass.

```bash
cargo test -p vibeflow --lib 2>&1 | tail -3
```

Expected: 135 + 17 = 152 default tests pass + 12 ignored.

```bash
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Both should be clean.

If clippy complains about `unused_must_use` on the `assert_eq!(...)` results, ignore — the macro consumes the return.

If clippy complains about `module_name_repetitions` or similar pedantic lint, that means clippy's lint level was raised somewhere; check the project clippy config and don't suppress inline.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/keymap.rs
git commit -m "feat(keymap): Shortcut enum + match_shortcut dispatch table (TDD)"
```

---

## Task 2: `mouse_encoder` — SGR + X10 mouse-event encoding (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/mouse_encoder.rs`

Pure logic. Encodes a click / release / drag at a given grid `Point` for a given `MouseButton` into the byte sequence the running app expects. SGR is used when the term has `TermMode::SGR_MOUSE` set; X10 is the legacy fallback.

- [ ] **Step 1: Write failing tests + encoder functions**

Replace the contents of `crates/vibeflow/src/render/mouse_encoder.rs` with:

```rust
//! Encodes mouse events into SGR (preferred) or X10 (legacy) escape
//! sequences for terminal mouse-mode passthrough into vim/htop/tmux/etc.
//!
//! All functions take 0-indexed grid coordinates from
//! `alacritty_terminal::index::Point`; SGR/X10 wire format expects
//! 1-indexed coordinates, so the encoders +1 internally.
//!
//! # SGR format (preferred — `TermMode::SGR_MOUSE` set)
//! `\x1b[<{button};{col};{row}M` for press / drag (capital M)
//! `\x1b[<{button};{col};{row}m` for release (lowercase m)
//!
//! Button codes:
//! * 0 = left, 1 = middle, 2 = right
//! * +32 (bit 5) = motion-while-pressed (drag)
//!
//! # X10 format (legacy — `SGR_MOUSE` clear)
//! `\x1b[M{btn+32}{col+32}{row+32}` — three raw bytes after the `M`. No
//! release distinction in pure X10; the modes that do drag tracking layer
//! it on, but at that point apps usually negotiate SGR.

use alacritty_terminal::index::Point;

/// Mouse button identifier. Matches `winit::event::MouseButton`'s three
/// canonical buttons; we don't pass through Other(_) buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

impl Button {
    fn code(self) -> u32 {
        match self {
            Button::Left => 0,
            Button::Middle => 1,
            Button::Right => 2,
        }
    }
}

/// Encode a press event. SGR uses the `M` terminator; X10 has no release
/// distinction so this is the same byte sequence as a release would be.
#[must_use]
pub fn encode_press(button: Button, point: Point, sgr: bool) -> Vec<u8> {
    if sgr {
        sgr_format(button.code(), point, b'M')
    } else {
        x10_format(button.code(), point)
    }
}

/// Encode a release event. SGR uses the `m` terminator (lowercase); X10
/// uses button code 3 (release marker).
#[must_use]
pub fn encode_release(button: Button, point: Point, sgr: bool) -> Vec<u8> {
    if sgr {
        sgr_format(button.code(), point, b'm')
    } else {
        x10_format(3, point) // X10's release sentinel
    }
}

/// Encode a drag (motion-while-pressed) event. Adds the +32 motion bit.
#[must_use]
pub fn encode_drag(button: Button, point: Point, sgr: bool) -> Vec<u8> {
    let code = button.code() + 32;
    if sgr {
        sgr_format(code, point, b'M')
    } else {
        x10_format(code, point)
    }
}

fn sgr_format(code: u32, point: Point, terminator: u8) -> Vec<u8> {
    // 1-indexed coordinates per the SGR spec.
    let col = (point.column.0 as u32) + 1;
    let row = (point.line.0 as u32) + 1;
    let body = format!("\x1b[<{code};{col};{row}");
    let mut out = body.into_bytes();
    out.push(terminator);
    out
}

fn x10_format(code: u32, point: Point) -> Vec<u8> {
    // 1-indexed + 32 offset. Caps at 255-32 = 223 cols/rows in pure X10.
    // For terminals beyond 223 the spec is ambiguous; SGR is the modern
    // workaround. We saturating-add for safety.
    let col_byte = (((point.column.0 as u32) + 1).saturating_add(32)).min(255) as u8;
    let row_byte = (((point.line.0 as u32) + 1).saturating_add(32)).min(255) as u8;
    let code_byte = (code.saturating_add(32)).min(255) as u8;
    vec![0x1b, b'[', b'M', code_byte, col_byte, row_byte]
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line};

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    // SGR tests — verbatim byte sequences against the spec.

    #[test]
    fn sgr_left_press_at_origin() {
        // (line 0, col 0) → 1-indexed → "1;1"
        assert_eq!(
            encode_press(Button::Left, pt(0, 0), true),
            b"\x1b[<0;1;1M".to_vec()
        );
    }

    #[test]
    fn sgr_left_release_uses_lowercase_m() {
        assert_eq!(
            encode_release(Button::Left, pt(5, 10), true),
            b"\x1b[<0;11;6m".to_vec()
        );
    }

    #[test]
    fn sgr_left_drag_adds_motion_bit() {
        // 0 (left) + 32 (motion) = 32
        assert_eq!(
            encode_drag(Button::Left, pt(0, 0), true),
            b"\x1b[<32;1;1M".to_vec()
        );
    }

    #[test]
    fn sgr_middle_button_code_is_1() {
        assert_eq!(
            encode_press(Button::Middle, pt(0, 0), true),
            b"\x1b[<1;1;1M".to_vec()
        );
    }

    #[test]
    fn sgr_right_button_code_is_2() {
        assert_eq!(
            encode_press(Button::Right, pt(0, 0), true),
            b"\x1b[<2;1;1M".to_vec()
        );
    }

    // X10 tests

    #[test]
    fn x10_left_press_at_origin() {
        // \x1b [ M (button+32) (col+1+32) (row+1+32)
        // button=0+32=32, col=1+32=33, row=1+32=33
        assert_eq!(
            encode_press(Button::Left, pt(0, 0), false),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
    }

    #[test]
    fn x10_release_uses_button_code_3() {
        // X10 release uses the special button code 3 (35 after +32 offset).
        assert_eq!(
            encode_release(Button::Left, pt(0, 0), false),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
    }

    #[test]
    fn x10_oversized_grid_saturates() {
        // Grid coord 250 → 1+250+32 = 283 → saturates to 255 in u8.
        let bytes = encode_press(Button::Left, pt(250, 250), false);
        assert_eq!(bytes[0..3], [0x1b, b'[', b'M']);
        assert_eq!(bytes[4], 255); // col saturates
        assert_eq!(bytes[5], 255); // row saturates
    }
}
```

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib mouse_encoder 2>&1 | tail -10
```

Expected: 8 tests pass.

```bash
cargo test -p vibeflow --lib 2>&1 | tail -3
```

Expected: 152 + 8 = 160 default + 12 ignored.

```bash
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Both clean.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/mouse_encoder.rs
git commit -m "feat(render): mouse_encoder — SGR + X10 encoding for mouse-mode passthrough (TDD)"
```

---

## Task 3: `Clipboard` — `arboard` wrapper

**Files:**
- Modify: `crates/vibeflow/src/clipboard.rs`

`arboard` requires a connection to the display server. On a headless test runner, `arboard::Clipboard::new()` may fail with `ContextCreationFailed`. We treat this as "no clipboard available" rather than a panic — vibeflow runs without copy/paste in that case.

- [ ] **Step 1: Implement the wrapper**

Replace the contents of `crates/vibeflow/src/clipboard.rs` with:

```rust
//! System clipboard wrapper. Stage 8 uses CLIPBOARD only; PRIMARY is deferred.
//!
//! `Clipboard` is owned by `App` (single instance per process). On systems
//! without a display server (CI, headless containers), `Clipboard::new()`
//! returns `Err`; the caller treats this as a soft failure — vibeflow runs
//! without copy/paste in that environment, but does not crash.

use anyhow::{Context, Result};

/// Wrapper over `arboard::Clipboard`. Only exposes the operations Stage 8
/// needs: copy a `&str`, paste a `String`. Errors are logged at `warn` by
/// the caller and do not crash.
pub struct Clipboard {
    inner: arboard::Clipboard,
}

impl Clipboard {
    /// Construct a new clipboard handle. Fails on headless systems
    /// (`arboard::Error::ContextCreationFailed`) — the caller should log and
    /// continue without clipboard support.
    ///
    /// # Errors
    /// Propagates `arboard` errors connecting to the display server.
    pub fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new()
            .context("create system clipboard handle (no display server?)")?;
        Ok(Self { inner })
    }

    /// Copy `text` to the system CLIPBOARD selector.
    ///
    /// # Errors
    /// Propagates `arboard` errors. The caller logs at `warn` and proceeds —
    /// a copy failure must not crash the renderer.
    pub fn copy(&mut self, text: &str) -> Result<()> {
        self.inner
            .set_text(text)
            .context("write to system clipboard")?;
        Ok(())
    }

    /// Paste from the system CLIPBOARD selector. Returns `None` if the
    /// clipboard is empty or holds non-text content (an image, etc.).
    pub fn paste(&mut self) -> Option<String> {
        self.inner.get_text().ok()
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
}
```

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 160 default tests + 13 ignored (the new ignored clipboard test).

LOCAL ONLY — verify the ignored test on host:

```bash
cargo test -p vibeflow --lib clipboard -- --ignored 2>&1 | tail -10
```

Expected on host (which has X11): 1 passed.
On a CI runner without DISPLAY: `Clipboard::new()` returns `Err` — `expect("clipboard available")` panics, the test is *expected* to fail in that env. That's why it's `#[ignore]`.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/clipboard.rs
git commit -m "feat(clipboard): arboard wrapper for system CLIPBOARD selector"
```

---

## Task 4: `SelectionTracker` state machine (TDD — biggest task)

**Files:**
- Modify: `crates/vibeflow/src/render/selection.rs`

The selection tracker is pure logic and the most complex component in Stage 8. ~20 unit tests cover every transition.

- [ ] **Step 1: Implement the types + state machine**

Replace the contents of `crates/vibeflow/src/render/selection.rs` with:

```rust
//! Mouse-driven cell selection state machine. Pure logic — no GPU, no winit,
//! no PTY dependency.
//!
//! Each `PtySession` owns one `SelectionTracker`. Mouse events from
//! `window.rs` are translated to grid `Point`s before reaching the tracker;
//! the tracker emits `Selection` updates that the renderer reads each frame.
//!
//! Selection lives in *grid* coordinates (alacritty `Point` — line, column).
//! The tracker doesn't care about pixels.
//!
//! State transitions:
//!
//! ```text
//! Idle ── mouse_down ──► Dragging
//!  ▲                       │
//!  │                       │ mouse_drag (updates `current`, snaps to mode)
//!  │                       │
//!  │                       ▼
//!  └─ mouse_up (and ───── Selected
//!     start==end &       (final, visible)
//!     count==1) or
//!     clear()
//! ```
//!
//! Word/Line modes are entered by raising the click counter via successive
//! mouse_down calls within 500ms and 1 cell of each other. Click counter
//! resets after 500ms gap or movement > 1 cell.

use std::time::{Duration, Instant};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;

const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);

/// Final or in-progress selection range. `start` and `end` are ordered such
/// that `start` is the visually-earlier point in reading order
/// (`start.line < end.line`, or same line with `start.column <= end.column`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Point,
    pub end: Point,
    pub mode: SelectionMode,
}

/// What kind of region the selection covers. Affects how `mouse_drag` snaps
/// the endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Cell-by-cell (single-click drag). Endpoints follow the mouse exactly.
    Cell,
    /// Word-bounded (double-click). Endpoints snap to word boundaries via
    /// `Term::semantic_search_left/right`.
    Word,
    /// Whole-line (triple-click). `start.column = 0`, `end.column = cols-1`.
    Line,
}

#[derive(Debug, Clone, Copy)]
struct ClickHistory {
    last_at: Instant,
    last_point: Point,
    count: u8,
}

/// Per-session state tracker. Owns the in-flight drag and the current
/// finalized selection (if any).
pub struct SelectionTracker {
    selection: Option<Selection>,
    drag_anchor: Option<Point>,
    click: Option<ClickHistory>,
}

impl Default for SelectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            selection: None,
            drag_anchor: None,
            click: None,
        }
    }

    /// Returns the current selection (in-flight or finalized) or `None`.
    #[must_use]
    pub fn current(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Returns true while the mouse is held and a drag is being built.
    /// Used by `window.rs` to decide whether `CursorMoved` should call
    /// `mouse_drag` or be ignored.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag_anchor.is_some()
    }

    /// Mouse-down handler. Internal API takes an `Instant` for testability;
    /// in production callers pass `Instant::now()`.
    pub fn mouse_down(
        &mut self,
        point: Point,
        shift_held: bool,
        term: &Term<VoidListener>,
        now: Instant,
    ) {
        // Click counter — distinguishes single / double / triple clicks.
        let count = self.bump_click(point, now);

        if shift_held && self.selection.is_some() {
            // Shift-extend: keep the existing start, move end to the new
            // point. Don't change mode. Don't change drag_anchor.
            if let Some(sel) = &mut self.selection {
                let (start, end) = order(sel.start, point);
                sel.start = start;
                sel.end = end;
            }
            self.drag_anchor = Some(self.selection.unwrap().start);
            return;
        }

        // Fresh selection. Mode follows the click count.
        let mode = match count {
            1 => SelectionMode::Cell,
            2 => SelectionMode::Word,
            3 => SelectionMode::Line,
            _ => SelectionMode::Cell, // 4th click wraps back to single
        };
        self.drag_anchor = Some(point);
        self.selection = Some(Selection { start: point, end: point, mode });
        // For Word/Line, immediately snap so a click without drag selects
        // the word/line under the cursor.
        self.snap_to_mode(term);
    }

    /// Mouse-drag handler. Updates the current endpoint based on `point`
    /// and re-snaps according to the active mode.
    pub fn mouse_drag(&mut self, point: Point, term: &Term<VoidListener>) {
        let Some(anchor) = self.drag_anchor else {
            return;
        };
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        let (start, end) = order(anchor, point);
        sel.start = start;
        sel.end = end;
        // Re-snap to keep word/line bounds consistent with the new endpoints.
        self.snap_to_mode(term);
    }

    /// Mouse-up handler. Finalizes the selection. If the user clicked
    /// without dragging (single-click, start==end), clears the selection.
    pub fn mouse_up(&mut self) {
        self.drag_anchor = None;
        let click_count = self.click.map(|c| c.count).unwrap_or(0);
        if let Some(sel) = self.selection {
            if sel.start == sel.end && click_count == 1 {
                self.selection = None;
            }
        }
    }

    /// Force-clear the selection. Called by `window.rs` on window resize,
    /// any input keystroke, etc.
    pub fn clear(&mut self) {
        self.selection = None;
        self.drag_anchor = None;
        // Click counter persists — clearing for "input typed" must not reset
        // the double-click window. The 500ms / 1-cell rule still gates it.
    }

    /// Yield each cell in the current selection in row-major order.
    /// Returns an empty iterator if no selection.
    pub fn cells<'a>(
        &'a self,
        term: &'a Term<VoidListener>,
    ) -> Box<dyn Iterator<Item = Point> + 'a> {
        let Some(sel) = self.selection else {
            return Box::new(std::iter::empty());
        };
        Box::new(cells_in_range(sel.start, sel.end, term.columns()))
    }

    /// Materialize the selection as a `String`. Returns `None` if there's
    /// no selection. Used by `Ctrl+Shift+C`.
    pub fn text(&self, term: &Term<VoidListener>) -> Option<String> {
        let sel = self.selection?;
        let mut out = String::new();
        let mut current_line = sel.start.line;
        for p in cells_in_range(sel.start, sel.end, term.columns()) {
            if p.line != current_line {
                out.push('\n');
                current_line = p.line;
            }
            // alacritty `Term::grid()[p].c` gives the character at the cell.
            // For empty cells it's typically `' '`.
            let cell = &term.grid()[p];
            out.push(cell.c);
        }
        Some(out)
    }

    fn bump_click(&mut self, point: Point, now: Instant) -> u8 {
        let count = match self.click {
            Some(prev)
                if now.duration_since(prev.last_at) <= MULTI_CLICK_WINDOW
                    && cell_distance(prev.last_point, point) <= 1 =>
            {
                prev.count.wrapping_add(1)
            }
            _ => 1,
        };
        self.click = Some(ClickHistory {
            last_at: now,
            last_point: point,
            count,
        });
        count
    }

    fn snap_to_mode(&mut self, term: &Term<VoidListener>) {
        let Some(sel) = self.selection.as_mut() else { return };
        match sel.mode {
            SelectionMode::Cell => {} // no snap
            SelectionMode::Word => {
                sel.start = term.semantic_search_left(sel.start);
                sel.end = term.semantic_search_right(sel.end);
            }
            SelectionMode::Line => {
                sel.start.column = Column(0);
                let last_col = term.columns().saturating_sub(1);
                sel.end.column = Column(last_col);
            }
        }
    }
}

/// Order two points so the result is `(earlier, later)` in reading order.
fn order(a: Point, b: Point) -> (Point, Point) {
    if (a.line, a.column.0) <= (b.line, b.column.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Manhattan-ish distance in cells. Used by the click-counter to decide if
/// two clicks are "close enough" to count as a double-click.
fn cell_distance(a: Point, b: Point) -> u32 {
    let line_diff = (a.line.0 - b.line.0).unsigned_abs();
    let col_diff = a.column.0.abs_diff(b.column.0) as u32;
    line_diff + col_diff
}

/// Iterate the cells covered by a selection from `start` to `end` (inclusive)
/// in linear text-flow order: end of `start.line` → all of `start.line+1` →
/// ... → start of `end.line`.
fn cells_in_range(
    start: Point,
    end: Point,
    cols: usize,
) -> impl Iterator<Item = Point> {
    // Single-line case
    if start.line == end.line {
        let line = start.line;
        let s = start.column.0;
        let e = end.column.0;
        return Box::new(
            (s..=e).map(move |c| Point::new(line, Column(c)))
        ) as Box<dyn Iterator<Item = Point>>;
    }
    // Multi-line case: a chain of three iterators (head, middle lines, tail).
    let last_col = cols.saturating_sub(1);
    let head = (start.column.0..=last_col)
        .map(move |c| Point::new(start.line, Column(c)));
    let middle_lines: Vec<Point> = ((start.line.0 + 1)..end.line.0)
        .flat_map(move |l| {
            (0..=last_col).map(move |c| Point::new(Line(l), Column(c)))
        })
        .collect();
    let tail = (0..=end.column.0)
        .map(move |c| Point::new(end.line, Column(c)));
    Box::new(head.chain(middle_lines.into_iter()).chain(tail))
        as Box<dyn Iterator<Item = Point>>
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config as TermConfig, Term};
    use std::time::Instant;

    fn make_term(cols: usize, lines: usize) -> Term<VoidListener> {
        let size = TermSize::new(cols, lines);
        Term::new(TermConfig::default(), &size, VoidListener)
    }

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    // Construction

    #[test]
    fn new_tracker_has_no_selection() {
        let t = SelectionTracker::new();
        assert!(t.current().is_none());
        assert!(!t.is_dragging());
    }

    // Single click (no drag)

    #[test]
    fn single_click_no_drag_clears_on_mouse_up() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(5, 10), false, &term, now);
        // Drag-anchor set; selection points to the single cell.
        assert!(t.is_dragging());
        assert_eq!(t.current().map(|s| s.start), Some(pt(5, 10)));
        t.mouse_up();
        // Click without drag → cleared.
        assert!(t.current().is_none());
        assert!(!t.is_dragging());
    }

    // Drag

    #[test]
    fn mouse_down_then_drag_then_up_finalizes_selection() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(2, 3), false, &term, now);
        t.mouse_drag(pt(2, 8), &term);
        t.mouse_up();
        let s = t.current().expect("selection finalized");
        assert_eq!(s.start, pt(2, 3));
        assert_eq!(s.end, pt(2, 8));
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    #[test]
    fn drag_endpoints_are_ordered_smaller_first() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        // Drag from (5, 20) to (2, 5) — backward.
        t.mouse_down(pt(5, 20), false, &term, now);
        t.mouse_drag(pt(2, 5), &term);
        let s = t.current().unwrap();
        assert_eq!(s.start, pt(2, 5));
        assert_eq!(s.end, pt(5, 20));
    }

    #[test]
    fn mouse_drag_without_prior_mouse_down_is_noop() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        t.mouse_drag(pt(0, 0), &term);
        assert!(t.current().is_none());
    }

    // Multi-click (word / line)

    #[test]
    fn double_click_within_window_extends_to_word_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, &term, t0);
        t.mouse_up();
        // Within 500ms and same point → count = 2 → Word mode.
        let t1 = t0 + Duration::from_millis(100);
        t.mouse_down(pt(0, 5), false, &term, t1);
        let s = t.current().unwrap();
        assert_eq!(s.mode, SelectionMode::Word);
    }

    #[test]
    fn triple_click_within_window_extends_to_line_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, &term, t0);
        t.mouse_up();
        t.mouse_down(pt(0, 5), false, &term, t0 + Duration::from_millis(50));
        t.mouse_up();
        t.mouse_down(pt(0, 5), false, &term, t0 + Duration::from_millis(100));
        let s = t.current().unwrap();
        assert_eq!(s.mode, SelectionMode::Line);
    }

    #[test]
    fn click_counter_resets_after_500ms_gap() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, &term, t0);
        t.mouse_up();
        // 600ms later — gap exceeds window.
        let t1 = t0 + Duration::from_millis(600);
        t.mouse_down(pt(0, 5), false, &term, t1);
        let s = t.current().unwrap();
        // Counter reset → count=1 → Cell mode.
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    #[test]
    fn click_counter_resets_when_point_moves() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, &term, t0);
        t.mouse_up();
        // Same time, different cell (>1 away).
        t.mouse_down(pt(0, 50), false, &term, t0 + Duration::from_millis(50));
        let s = t.current().unwrap();
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    // Shift-extend

    #[test]
    fn shift_click_extends_existing_selection() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(2, 5), false, &term, now);
        t.mouse_drag(pt(2, 10), &term);
        t.mouse_up();
        // Shift-click further out — should extend the end.
        t.mouse_down(pt(2, 20), true, &term, now + Duration::from_millis(50));
        let s = t.current().unwrap();
        assert_eq!(s.start, pt(2, 5));
        assert_eq!(s.end, pt(2, 20));
    }

    // Clear

    #[test]
    fn clear_drops_selection_and_drag_anchor() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(0, 5), false, &term, now);
        t.mouse_drag(pt(0, 10), &term);
        // Mid-drag clear (e.g. user typed something).
        t.clear();
        assert!(t.current().is_none());
        assert!(!t.is_dragging());
    }

    // Cells iteration

    #[test]
    fn single_line_selection_yields_left_to_right_cells() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(0, 3), false, &term, now);
        t.mouse_drag(pt(0, 6), &term);
        t.mouse_up();
        let cells: Vec<Point> = t.cells(&term).collect();
        assert_eq!(
            cells,
            vec![pt(0, 3), pt(0, 4), pt(0, 5), pt(0, 6)]
        );
    }

    #[test]
    fn multi_line_selection_wraps_around_end_of_line() {
        let mut t = SelectionTracker::new();
        let term = make_term(5, 24); // 5 cols
        let now = Instant::now();
        t.mouse_down(pt(0, 3), false, &term, now);
        t.mouse_drag(pt(2, 1), &term);
        t.mouse_up();
        let cells: Vec<Point> = t.cells(&term).collect();
        assert_eq!(
            cells,
            vec![
                pt(0, 3), pt(0, 4),                  // tail of line 0
                pt(1, 0), pt(1, 1), pt(1, 2), pt(1, 3), pt(1, 4), // all of line 1
                pt(2, 0), pt(2, 1),                  // head of line 2
            ]
        );
    }

    #[test]
    fn empty_tracker_yields_no_cells() {
        let t = SelectionTracker::new();
        let term = make_term(80, 24);
        assert_eq!(t.cells(&term).count(), 0);
    }
}
```

- [ ] **Step 2: Verify**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib selection 2>&1 | tail -20
```

Expected: 14 tests pass.

```bash
cargo test -p vibeflow --lib 2>&1 | tail -3
```

Expected: 160 + 14 = 174 default tests pass + 13 ignored.

```bash
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Both clean.

NOTE: If `Term::grid()` returns a `Grid<Cell>` that doesn't implement `Index<Point>` directly, you may need `&term.grid()[p.line][p.column]` instead of `&term.grid()[p]`. Check the alacritty_terminal API at `~/.cargo/registry/src/.../alacritty_terminal-0.24.2/src/grid/mod.rs` if this fails.

If `TermSize` isn't in scope, the import path is `alacritty_terminal::term::test::TermSize` (it's gated on `cfg(test)` in some versions; if so, copy the simple `TermSize` impl from session.rs which has a runtime helper).

Some alacritty_terminal Grid index APIs require `&grid[Line(n)][Column(m)]`; verify by reading the existing `quad.rs` `build_cell_instances` for how it accesses cells: `cell.c` from `content.display_iter`.

If `TermSize` is private, define a local helper in the test module:

```rust
struct LocalTermSize { columns: usize, screen_lines: usize }
impl alacritty_terminal::term::Dimensions for LocalTermSize {
    fn columns(&self) -> usize { self.columns }
    fn screen_lines(&self) -> usize { self.screen_lines }
    fn total_lines(&self) -> usize { self.screen_lines }
}
fn make_term(cols: usize, lines: usize) -> Term<VoidListener> {
    let size = LocalTermSize { columns: cols, screen_lines: lines };
    Term::new(TermConfig::default(), &size, VoidListener)
}
```

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/selection.rs
git commit -m "feat(render): SelectionTracker state machine + multi-click + word/line snap (TDD)"
```

---

## Task 5: `PtySession::restart` + `App::restart_active` + `App::cycle_active`

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`
- Modify: `crates/vibeflow/src/app.rs`

`PtySession` gains a `selection: SelectionTracker` field and a `restart` method. `App` gets two helpers used by the keyboard dispatch.

- [ ] **Step 1: Add `SelectionTracker` to `PtySession`**

Open `crates/vibeflow/src/session/session.rs`. Find the `PtySession` struct definition (around line 78). Add the field at the end:

```rust
pub struct PtySession {
    rx: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    dispatcher: OscDispatcher,
    parser: Processor,
    term: Term<VoidListener>,
    tracker: AiStateTracker,
    alive: bool,
    label: TabLabel,
    bell_pending: bool,
    /// Per-tab mouse-driven cell selection. Stage 8.
    pub selection: crate::render::selection::SelectionTracker,
}
```

The visibility is `pub` so `window.rs`'s mouse handlers can call `session.selection.mouse_down(...)` directly. Internal mutation via `.selection.method()` is fine; we don't need a getter.

In the `Self { ... }` initializer at the bottom of `PtySession::spawn`, add:

```rust
selection: crate::render::selection::SelectionTracker::new(),
```

Place it as the last field initializer (matches the struct order).

- [ ] **Step 2: Add `restart()` to `PtySession`**

Append to the `impl PtySession` block, after `set_label` and before the test module:

```rust
    /// Re-spawn the session in place. Kills the existing child (if alive),
    /// drops the old receiver, and replaces `*self` with a fresh `spawn`
    /// running `$SHELL` (fallback `bash`). Preserves the current PTY size
    /// by re-applying it after the new spawn — avoids the new shell
    /// believing it's at the hardcoded `DEFAULT_COLS`/`DEFAULT_ROWS`.
    ///
    /// Stage 8 always uses `$SHELL` regardless of the dying process. Stage
    /// 9 (TOML config) may grow argv-replay if a clear use case emerges.
    /// Tracker config also resets to default; Stage 9's TOML hot-reload
    /// will pass the current user config through.
    ///
    /// # Errors
    /// Propagates spawn / IO errors.
    pub fn restart(&mut self) -> std::io::Result<()> {
        let _ = self.child.kill();
        // Capture the current PTY size before we drop the old master.
        let size = self.master.get_size().ok();
        // The reader thread sees its tx invalidated when the new spawn
        // replaces self; we don't need to join it explicitly.
        let argv = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
        let mut new_session = PtySession::spawn(&[argv.as_str()], TrackerConfig::default())?;
        if let Some(s) = size {
            // PtySize uses u16 for rows/cols. Re-apply to the new master.
            let _ = new_session.resize(s.rows, s.cols);
        }
        *self = new_session;
        Ok(())
    }
```

- [ ] **Step 3: Add a unit test for restart**

Append to `mod tests` in `session.rs` (the existing test module — DO NOT MODIFY EXISTING TESTS):

```rust
    #[test]
    fn restart_replaces_internals_with_fresh_spawn() {
        // Spawn a sleep then restart. After restart, the new session must
        // be alive (the new shell is freshly spawned).
        let mut s = PtySession::spawn(&["sleep", "10"], TrackerConfig::default())
            .expect("first spawn");
        s.restart().expect("restart");
        // Give the new PTY a moment to initialize before the liveness check
        // — `child.try_wait` can race against the spawn handshake on slower
        // CI runners.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(s.is_alive(), "restarted session should be alive");
        // Send some bytes to confirm the new PTY is responsive.
        s.send_input(b"\n").expect("send_input on restarted session");
        // Drop the session — its Drop impl (or the kill-on-drop the
        // child handle has) cleans up the spawned shell.
        drop(s);
    }
```

- [ ] **Step 4: Add `restart_active` and `cycle_active` to `App`**

Open `crates/vibeflow/src/app.rs`. Append to the `impl App` block:

```rust
    /// Mutable slice of all sessions. Stage 8's selection / mouse routing
    /// needs `tabs_mut().get_mut(active)` to call `selection.mouse_*` and
    /// `send_input` from the `window.rs` dispatch layer.
    #[must_use]
    pub fn tabs_mut(&mut self) -> &mut [PtySession] {
        &mut self.tabs
    }

    /// Restart the dead active session. No-op on live sessions and on the
    /// no-tabs sentinel state.
    ///
    /// # Errors
    /// Propagates `PtySession::restart` errors.
    pub fn restart_active(&mut self) -> std::io::Result<()> {
        let Some(s) = self.tabs.get_mut(self.active) else { return Ok(()) };
        if s.is_alive() {
            tracing::trace!("Ctrl+Shift+R on live tab; ignoring");
            return Ok(());
        }
        s.restart()
    }

    /// Cycle the active tab by `direction`: +1 = forward, -1 = backward.
    /// Wraps around. No-op when there are no tabs.
    pub fn cycle_active(&mut self, direction: i32) {
        let len = self.tabs.len();
        if len == 0 {
            return;
        }
        let cur = self.active as i32;
        let next = (cur + direction).rem_euclid(len as i32);
        self.active = next as usize;
    }
```

`tracing` is already a vibeflow dependency (used throughout `window.rs` for tab spawn / state-changed / bell / died logs).

- [ ] **Step 5: Add a unit test for cycle_active**

Append to `mod tests` in `app.rs`:

```rust
    #[test]
    fn cycle_active_wraps_forward_and_backward() {
        let mut app = App::new();
        // App::new() starts empty. App::new_tab spawns a sleep then sets
        // active. Use it three times to populate.
        for _ in 0..3 {
            app.new_tab(&["sleep", "30"]).expect("new_tab spawns");
        }
        app.set_active(0);
        app.cycle_active(1);
        assert_eq!(app.active(), 1);
        app.cycle_active(1);
        assert_eq!(app.active(), 2);
        app.cycle_active(1);
        assert_eq!(app.active(), 0); // wraps
        app.cycle_active(-1);
        assert_eq!(app.active(), 2); // wraps backward
    }
```

NOTE: This test spawns three real `sleep 30` processes via PTY. They get cleaned up when `app` drops at end-of-test (PtySession's Drop / child handle terminates them). If running this test starts hanging on slow runners, mark it `#[ignore]` and rely on the manual smoke walkthrough — the cycle math is trivial `rem_euclid` and doesn't strictly need an integration test.

- [ ] **Step 6: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: 174 + 1 or 2 new tests = 175-176 default + 13 ignored.

If `restart_replaces_internals_with_fresh_spawn` fails because spawning `sleep` is slow on the first run, try increasing the default test timeout or adding a 100ms sleep to let the new PTY initialize:

```rust
std::thread::sleep(std::time::Duration::from_millis(100));
```

before the `assert!(s.is_alive())`.

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/session.rs \
        crates/vibeflow/src/session/tracker.rs \
        crates/vibeflow/src/app.rs
git commit -m "feat(session): PtySession::restart + App::restart_active/cycle_active"
```

If you didn't modify `tracker.rs`, omit it.

---

## Task 6: Wire `window.rs` — keyboard shortcut dispatch

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

Insert `keymap::match_shortcut` *before* the existing typed-input fallthrough so a matched shortcut suppresses the literal byte.

- [ ] **Step 1: Read the existing keyboard dispatch**

```bash
grep -n "WindowEvent::KeyboardInput\|fn map_key\|map_key_to_bytes\|send_input" crates/vibeflow/src/window.rs | head -10
```

Note the line numbers of:
- The `KeyboardInput` arm in `window_event` (the function that handles `WindowEvent`).
- The current `match` block that translates winit `Key` → bytes for `send_input`.

- [ ] **Step 2: Add the `Clipboard` field to `App` (or wherever the `App` lives in window.rs)**

`App` is constructed in `Window::new` (or similar). Find that constructor. Add a `clipboard: Option<crate::clipboard::Clipboard>` field if `App` doesn't already own it. Initialize:

```rust
let clipboard = match crate::clipboard::Clipboard::new() {
    Ok(c) => Some(c),
    Err(e) => {
        tracing::warn!("system clipboard unavailable: {e}");
        None
    }
};
```

If `App` is the right owner, put the field on `App`. If `Window` is the right owner (because the dispatch is in `window.rs`), put it on `Window`. Pick whichever has fewer threading complications — both are fine.

For Stage 8, place it on `Window` for simplicity: `Window` already owns the wgpu Renderer, so adding one more singleton is local.

- [ ] **Step 3: Insert the shortcut dispatch in the KeyboardInput arm**

Find the `WindowEvent::KeyboardInput` arm. The structure is roughly:

```rust
WindowEvent::KeyboardInput { event: ke, .. } => {
    if ke.state == ElementState::Pressed {
        // existing key→bytes mapping ...
        if let Some(bytes) = map_key_to_bytes(&ke.logical_key, &self.modifiers) {
            self.app.active_tab_mut().map(|s| s.send_input(&bytes));
        }
    }
}
```

Replace the body with shortcut-first dispatch:

```rust
WindowEvent::KeyboardInput { event: ke, .. } => {
    if ke.state != ElementState::Pressed {
        return;
    }
    // Shortcut dispatch FIRST. If the combo matches, suppress the literal
    // byte fallthrough.
    if let Some(shortcut) =
        crate::keymap::match_shortcut(&ke.logical_key, self.current_modifiers)
    {
        self.handle_shortcut(shortcut);
        return;
    }
    // Otherwise: typed-input fallthrough. Selection clears on any input.
    if let Some(s) = self.app.tabs_mut().get_mut(self.app.active()) {
        s.selection.clear();
    }
    if let Some(bytes) = map_key_to_bytes(&ke.logical_key, &self.modifiers) {
        if let Some(s) = self.app.tabs_mut().get_mut(self.app.active()) {
            let _ = s.send_input(&bytes);
        }
    }
}
```

NOTE: `self.current_modifiers` returns `ModifiersState` (the bitfield). winit 0.30's `Modifiers` struct wraps `state()` accessor. Double-check by reading existing code in `window.rs`.

`App::tabs_mut` is added as part of Task 5 alongside `restart_active` / `cycle_active`. If you're executing Task 6 before Task 5 for some reason, add it now too — three lines, see Task 5 Step 4.

- [ ] **Step 4: Implement `handle_shortcut`**

Add to the `impl Window` (or wherever the dispatch struct is) block:

```rust
    fn handle_shortcut(&mut self, shortcut: crate::keymap::Shortcut) {
        use crate::keymap::Shortcut;
        match shortcut {
            Shortcut::NewTab => {
                // `App::new_tab` spawns + appends + sets active in one call.
                let argv = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
                if let Err(e) = self.app.new_tab(&[argv.as_str()]) {
                    tracing::warn!("new tab spawn failed: {e}");
                }
            }
            Shortcut::CloseTab => {
                self.app.close_tab(self.app.active());
            }
            Shortcut::NextTab => self.app.cycle_active(1),
            Shortcut::PrevTab => self.app.cycle_active(-1),
            Shortcut::RestartTab => {
                if let Err(e) = self.app.restart_active() {
                    tracing::warn!("restart failed: {e}");
                }
            }
            Shortcut::Copy => self.handle_copy(),
            Shortcut::Paste => self.handle_paste(),
        }
    }

    fn handle_copy(&mut self) {
        let Some(clipboard) = self.clipboard.as_mut() else { return };
        let Some(s) = self.app.tabs().get(self.app.active()) else { return };
        let Some(text) = s.selection.text(s.term()) else { return };
        if let Err(e) = clipboard.copy(&text) {
            tracing::warn!("copy failed: {e}");
        }
    }

    fn handle_paste(&mut self) {
        let Some(clipboard) = self.clipboard.as_mut() else { return };
        let Some(text) = clipboard.paste() else { return };
        let Some(s) = self.app.tabs_mut().get_mut(self.app.active()) else { return };
        let bracketed = s
            .term()
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        if bracketed {
            let _ = s.send_input(b"\x1b[200~");
            let _ = s.send_input(text.as_bytes());
            let _ = s.send_input(b"\x1b[201~");
        } else {
            let _ = s.send_input(text.as_bytes());
        }
    }
```

NOTE: `App::close_tab` is `pub fn close_tab(&mut self, idx: usize)` (no Result) — verified against current `app.rs`. The plan's match arm matches.

- [ ] **Step 5: Verify build green**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean, test count unchanged from Task 5 (no new tests added in Task 6).

Smoke run is for Task 9.

- [ ] **Step 6: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): keyboard shortcut dispatch + copy/paste/restart handlers"
```

---

## Task 7: Wire `window.rs` — mouse event routing

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

Mouse events route to either the `SelectionTracker` (when mouse mode is off OR Shift held) or the PTY (encoded via `mouse_encoder`).

- [ ] **Step 1: Read the existing mouse dispatch**

```bash
grep -n "MouseInput\|CursorMoved\|cursor_pos\|MouseButton\|mouse_event\|tab_bar_hit" crates/vibeflow/src/window.rs | head -15
```

Note the existing arms for `WindowEvent::MouseInput` and `WindowEvent::CursorMoved`.

- [ ] **Step 2: Add a `pixel_to_grid_point` helper**

If `window.rs` doesn't already have a helper translating pixel coordinates to alacritty `Point`, add it. The Stage 6 `pixels_to_grid` helper produces `(row, col)` u32 tuples — adapt:

```rust
fn pixel_to_grid_point(
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
    px: u32,
    py: u32,
) -> Option<alacritty_terminal::index::Point> {
    use alacritty_terminal::index::{Column, Line, Point};
    if py < bar_height_px {
        return None; // tab bar — selection is grid-only
    }
    let py = py - bar_height_px;
    let col = (px / cell_w) as usize;
    let line = (py / cell_h) as i32;
    Some(Point::new(Line(line), Column(col)))
}
```

- [ ] **Step 3: Update `WindowEvent::MouseInput` arm**

Find the existing arm. Replace its body with:

```rust
WindowEvent::MouseInput { state, button, .. } => {
    let Some((px, py)) = self.cursor_pos else { return };
    let pressed = matches!(state, ElementState::Pressed);
    let released = matches!(state, ElementState::Released);
    let shift = self.current_modifiers.shift_key();

    // Tab bar passthrough (existing Stage 6 hit-test). The y-bound check
    // is done inside `tab_bar_hit_test`.
    if py < self.layout_bar_height_px() {
        // existing tab_bar handling — leave as-is. The early-return below
        // applies only when we DIDN'T fall into the tab bar path.
        // (Adapt to match the actual existing arm structure — this is
        // pseudocode for the merge.)
        // ... existing handling ...
        return;
    }

    let (cell_w, cell_h) = self.cell_metrics();
    let Some(point) = pixel_to_grid_point(
        cell_w, cell_h, self.layout_bar_height_px(), px, py,
    ) else {
        return;
    };

    let Some(s) = self.app.tabs_mut().get_mut(self.app.active()) else {
        return;
    };
    let mode_on = s.term().mode().intersects(
        alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
            | alacritty_terminal::term::TermMode::MOUSE_DRAG
            | alacritty_terminal::term::TermMode::MOUSE_MOTION,
    );
    let sgr = s.term().mode().contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
    let encoder_button = match button {
        winit::event::MouseButton::Left => Some(crate::render::mouse_encoder::Button::Left),
        winit::event::MouseButton::Middle => Some(crate::render::mouse_encoder::Button::Middle),
        winit::event::MouseButton::Right => Some(crate::render::mouse_encoder::Button::Right),
        _ => None,
    };

    if mode_on && !shift {
        // Pass to PTY as encoded mouse event.
        if let Some(b) = encoder_button {
            let bytes = if pressed {
                crate::render::mouse_encoder::encode_press(b, point, sgr)
            } else if released {
                crate::render::mouse_encoder::encode_release(b, point, sgr)
            } else {
                return;
            };
            let _ = s.send_input(&bytes);
        }
        return;
    }

    // Selection path — only Left button creates / clears selection. Right
    // and Middle buttons are no-ops in the selection world (they only
    // activate when mouse mode is on).
    if button != winit::event::MouseButton::Left {
        return;
    }
    if pressed {
        let now = std::time::Instant::now();
        s.selection.mouse_down(point, shift, s.term(), now);
    } else if released {
        s.selection.mouse_up();
    }
}
```

NOTE: `self.layout_bar_height_px()` is hypothetical — use whatever the existing accessor is for the cell-grid y-offset (likely `tab_bar_height_px(cell_h)` from `tabs.rs`). Same for `self.cell_metrics()` — read existing code.

The `// existing tab_bar handling` block is a placeholder for whatever Stage 6's tab-bar hit-test code does. Don't delete it; just integrate the new logic for the cell-grid region after it.

- [ ] **Step 4: Update `WindowEvent::CursorMoved` arm**

```rust
WindowEvent::CursorMoved { position, .. } => {
    let (px, py) = (position.x as u32, position.y as u32);
    self.cursor_pos = Some((px, py));

    if py < self.layout_bar_height_px() {
        // tab bar hover — existing Stage 6 handling.
        return;
    }

    let (cell_w, cell_h) = self.cell_metrics();
    let Some(point) = pixel_to_grid_point(
        cell_w, cell_h, self.layout_bar_height_px(), px, py,
    ) else { return };

    let shift = self.current_modifiers.shift_key();
    let Some(s) = self.app.tabs_mut().get_mut(self.app.active()) else { return };

    let mode_on = s.term().mode().intersects(
        alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
            | alacritty_terminal::term::TermMode::MOUSE_DRAG
            | alacritty_terminal::term::TermMode::MOUSE_MOTION,
    );
    let sgr = s.term().mode().contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
    let drag_tracking = s.term().mode().intersects(
        alacritty_terminal::term::TermMode::MOUSE_DRAG
            | alacritty_terminal::term::TermMode::MOUSE_MOTION,
    );

    if mode_on && drag_tracking && !shift {
        // Encode as drag for PTY. v0.1 simplification: assume Left button
        // drags. Middle/right drags in mouse-mode-aware apps are extremely
        // rare and Stage 9 can track which button initiated the drag if
        // that ever bites.
        let bytes = crate::render::mouse_encoder::encode_drag(
            crate::render::mouse_encoder::Button::Left,
            point,
            sgr,
        );
        let _ = s.send_input(&bytes);
    } else if s.selection.is_dragging() {
        s.selection.mouse_drag(point, s.term());
    }
}
```

- [ ] **Step 5: Resize / typing clears selection**

Find the `WindowEvent::Resized` arm. After the existing resize logic, add:

```rust
for tab in self.app.tabs_mut().iter_mut() {
    tab.selection.clear();
}
```

The keystroke-clears-selection logic was added in Task 6 Step 3 already.

- [ ] **Step 6: Verify build green**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -10
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean, all tests pass.

The build may surface borrow-checker errors around the temporary `s = self.app.tabs_mut().get_mut(...)` followed by calls that need `&self.app` (immutable). If so, refactor to take the `cell_w/cell_h/bar_height` values up front (before borrowing `app` mutably), do the term-state read into a local, then call the selection method with the locals.

If clippy complains about `too_many_lines` in `window_event`, that's because the new arms grew the function. Either factor into helper methods (`fn handle_mouse_input` / `fn handle_cursor_moved`) or add `#[allow(clippy::too_many_lines)]` at the function level — the precedent for clippy::* allows in this codebase is established.

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): mouse event routing — selection + mouse-mode passthrough"
```

---

## Task 8: Render selection rects

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

The active tab's selection becomes a list of `RectInstance`s appended to the unified rect buffer between tab rects and banner rect. One new `tab_bar_pipeline.draw_range(...)` call.

- [ ] **Step 1: Add a `build_selection_rects` helper**

In `crates/vibeflow/src/render/mod.rs`, near the other build helpers (or as a free function), add:

```rust
const SELECTION_COLOR: [f32; 4] = [0.4, 0.6, 1.0, 0.4];

fn build_selection_rects(
    selection: &crate::render::selection::SelectionTracker,
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
) -> Vec<crate::render::tabs::RectInstance> {
    selection
        .cells(term)
        .map(|p| {
            let screen_x = (p.column.0 as u32 * cell_w) as f32;
            let screen_y = (p.line.0 as u32 * cell_h + bar_height_px) as f32;
            crate::render::tabs::RectInstance::new(
                screen_x,
                screen_y,
                cell_w as f32,
                cell_h as f32,
                SELECTION_COLOR,
            )
        })
        .collect()
}
```

If `p.line.0 as u32` triggers a clippy `cast_sign_loss` warning, use `p.line.0.max(0) as u32`. Selection points whose `line < 0` (scrollback) shouldn't appear in v0.1 (selection is visible-grid-only); we filter them out:

```rust
.filter_map(|p| {
    if p.line.0 < 0 { return None; }
    Some(crate::render::tabs::RectInstance::new(
        (p.column.0 as u32 * cell_w) as f32,
        (p.line.0 as u32 * cell_h + bar_height_px) as f32,
        cell_w as f32,
        cell_h as f32,
        SELECTION_COLOR,
    ))
})
.collect()
```

- [ ] **Step 2: Integrate selection rects into `Renderer::render`**

Find the section in `Renderer::render` (post-Stage-7.5) that builds `tab_rects` and the banner / bell rects. Insert the selection-rect building between the active session lookup and the rect concatenation:

```rust
// After tab_rects = self.tab_bar.build_rects(app, &layout); and similar setup.

let selection_rects = if let Some(active) = app.tabs().get(app.active()) {
    if let Some(_) = active.selection.current() {
        build_selection_rects(
            &active.selection,
            active.term(),
            cell_w,
            cell_h,
            layout.bar_height_px,
        )
    } else {
        Vec::new()
    }
} else {
    Vec::new()
};
```

Then update the offset bookkeeping (the post-Stage-7.5 unified rect buffer):

```rust
let tab_rect_count = tab_rects.len() as u32;
let selection_rect_offset = tab_rect_count;
let selection_rect_count = selection_rects.len() as u32;
let banner_rect_offset = selection_rect_offset + selection_rect_count;
let banner_rect_count = u32::from(banner_rect.is_some());
let bell_rect_offset = banner_rect_offset + banner_rect_count;
let bell_rect_count = u32::from(bell_rect.is_some());
let total_rects = bell_rect_offset + bell_rect_count;
```

And update `all_rects` extension order:

```rust
let mut all_rects = Vec::with_capacity(total_rects as usize);
all_rects.extend_from_slice(&tab_rects);
all_rects.extend_from_slice(&selection_rects);
if let Some(r) = banner_rect {
    all_rects.push(r);
}
if let Some(r) = bell_rect {
    all_rects.push(r);
}
```

- [ ] **Step 3: Add a `draw_range` call for selection in the render pass**

Inside the `{ let mut pass = encoder.begin_render_pass(...) ... }` block, add a call after the tab text and before the banner block:

```rust
// ---- Selection highlights ----
self.tab_bar_pipeline
    .draw_range(&mut pass, selection_rect_offset..banner_rect_offset);
```

Place it AFTER tab text (so selection draws over cells but tab text stays untouched, since they don't overlap anyway), and BEFORE the banner block. The full draw order becomes:

```
1. quad_pipeline.draw_range(0..cell_count)                 — cells
2. tab_bar_pipeline.draw_range(0..tab_rect_count)          — tab rects
3. quad_pipeline.draw_range(tab_glyph_offset..banner_glyph_offset)
                                                           — tab text
4. tab_bar_pipeline.draw_range(selection_rect_offset..banner_rect_offset)
                                                           — selection (NEW)
5. tab_bar_pipeline.draw_range(banner_rect_offset..bell_rect_offset)
                                                           — banner rect
6. quad_pipeline.draw_range(banner_glyph_offset..total_quads)
                                                           — banner glyphs
7. tab_bar_pipeline.draw_range(bell_rect_offset..total_rects)
                                                           — bell flash
```

- [ ] **Step 4: Add an ignored test for `build_selection_rects`**

Append to `mod tests` if one exists in `render/mod.rs`, or skip this step if no test module exists there. The test pattern is small:

```rust
#[cfg(test)]
mod selection_rect_tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config as TermConfig, Term};
    use crate::render::selection::SelectionTracker;
    use std::time::Instant;

    #[test]
    #[ignore = "depends on alacritty_terminal::term::test::TermSize visibility"]
    fn build_selection_rects_emits_one_per_cell() {
        let size = TermSize::new(80, 24);
        let term = Term::new(TermConfig::default(), &size, VoidListener);
        let mut t = SelectionTracker::new();
        t.mouse_down(Point::new(Line(0), Column(2)), false, &term, Instant::now());
        t.mouse_drag(Point::new(Line(0), Column(5)), &term);
        t.mouse_up();
        let rects = build_selection_rects(&t, &term, 10, 20, 50);
        assert_eq!(rects.len(), 4); // cols 2..=5 → 4 cells
    }
}
```

If `TermSize::new` is gated and inaccessible from this module, mark the whole block `#[cfg(test)]` and skip it; the smoke walkthrough is the visual gate.

- [ ] **Step 5: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build green. Test count = ~176 default + 13-14 ignored.

Smoke run on host:

```bash
cd /path/to/vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow &
```

Test:
- Drag-select a chunk of the prompt — should see a blue highlight.
- Single-click somewhere — highlight clears.
- Press Ctrl+Shift+T — new tab opens. Old tab's selection persists when you switch back.

If selection rendering shows weird artifacts (extra cells, wrong color), check the `RectInstance::new` argument order against the existing tab-rect calls.

- [ ] **Step 6: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(render): selection rect rendering — overlay highlight in unified rect buffer"
```

---

## Task 9: Final verification + smoke checklist + tag

- [ ] **Step 1: Append Stage 8 section to `docs/TESTING.md`**

After the Stage 7.5 section, append:

```markdown

## Stage 8 — clipboard + keyboard shortcuts + selection

Run:

```bash
cd /path/to/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

### Keyboard shortcuts

- [ ] `Ctrl+Shift+T` opens a new tab with `$SHELL`. Tab bar shows two tabs.
- [ ] `Super+T` (Cmd+T over VNC from Mac) also opens a new tab.
- [ ] `Ctrl+Shift+W` closes the active tab. Continue closing until 1 tab remains; one more close leaves the window with no tabs.
- [ ] `Ctrl+Shift+T` from the no-tabs state spawns a fresh tab.
- [ ] `Ctrl+Tab` cycles forward; `Ctrl+Shift+Tab` cycles backward. Wraps at the ends.
- [ ] `Super+Tab` and `Super+Shift+Tab` also cycle. (May be grabbed by the WM — note for Stage 9 if it's flaky.)
- [ ] `Ctrl+C` at a shell prompt sends SIGINT (interrupts a `sleep 100`). Ctrl+C is NOT remapped to copy.
- [ ] `Ctrl+V` at a shell prompt is `quoted-insert` (next char is literal — not paste).

### Mouse selection

- [ ] Drag from one cell to another → blue 40%-alpha highlight rendered between the two points.
- [ ] Drag spanning multiple lines → highlight wraps around line ends.
- [ ] Single-click somewhere → prior selection clears, no new selection rendered.
- [ ] Double-click on a word → only that word highlights (snapped to whitespace + punctuation boundaries).
- [ ] Triple-click on a line → entire line highlights.
- [ ] Shift+click after an existing selection extends the end without losing the start.

### Clipboard

- [ ] Drag-select "(base) bhengen", press `Ctrl+Shift+C`. Paste into another GUI app on host (e.g., Firefox URL bar) — should arrive as text.
- [ ] In Firefox, copy "hello world" with `Ctrl+C`. Switch back to vibeflow. Press `Ctrl+Shift+V` → "hello world" appears at the prompt.
- [ ] Copy a multi-line `for` loop:
   ```bash
   for i in 1 2 3
   do
     echo $i
   done
   ```
   from another app. Paste into vibeflow at a `bash` prompt with `Ctrl+Shift+V`. Bash should NOT execute each line separately — it should arrive as a single editable buffer (visible via the `>` continuation prompt). Pressing Enter at the end then runs the whole thing.
- [ ] `Super+C` and `Super+V` also work (or are silently grabbed by WM — note if so).

### Mouse mode passthrough

- [ ] Run `vim` and `:set mouse=a`. Click in the buffer — vim's cursor moves to that location. (Mouse events reach vim.)
- [ ] In `vim`, press and hold Shift while dragging — vibeflow should select the text *across vim's display*, ignoring vim's mouse mode. Release Shift, click without Shift — vim again sees the click.
- [ ] In `htop`, click on a process row — htop should highlight it. (Mouse events reach htop.)
- [ ] In `tmux`, mouse mode behavior unchanged from upstream tmux's expectations.

### Restart dead session

- [ ] In a tab, press `Ctrl+D`. Banner appears with "session died -- press Ctrl+Shift+R to retry".
- [ ] Press `Ctrl+Shift+R`. Banner disappears, fresh `bash` prompt appears.
- [ ] Press `Ctrl+Shift+R` on a *live* tab → no-op. Tab stays untouched.

### Selection persistence

- [ ] Drag-select in tab A. Press `Ctrl+Tab` to switch to tab B. Press `Ctrl+Shift+Tab` to come back to tab A — selection still highlighted.
- [ ] Type a key in tab A — selection clears.
- [ ] Resize the window — selection clears.

### Cross-cutting

- [ ] `vi` enters and exits cleanly with mouse=a; cursor blink continues correctly post-Stage-7.
- [ ] Re-run with `WINIT_UNIX_BACKEND=x11` — all checks above still pass.

**Known Stage 8 limitations (deferred to later stages):**

- PRIMARY clipboard / middle-click paste is not wired (CLIPBOARD only). Stage 9.
- Right-click does not open a context menu — Stage 9 / 10 (needs overlay rendering).
- Block (column) selection (Alt+drag) — Stage 9.
- Configurable shortcuts and selection color — Stage 9 (TOML config).
- Selection in scrollback — Stage 10 (depends on scrollback rendering).
- Selection that anchors to grid content (survives scroll in background tabs) — open-ended; revisit if it bites in practice.
- Some smiley-face emoji (U+1F600..) still resolve to DejaVu Sans rather than Noto Color Emoji on Ubuntu; that's a font priority issue from Stage 7.5 deferred to Stage 9.
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
- ~176 default lib tests pass + ~14 ignored.
- 27 protocol crate tests, 15 npm tests, integration tests pass.
- The exact total varies by harness count. The gate is "any failure stops" — exact numbers are diagnostic, not gating.

If any test fails, STOP and report.

- [ ] **Step 3: 60-second fuzz on the protocol parser**

```bash
cd /path/to/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: no crashes / leaks. Same as Stage 7.5's fuzz baseline (~13M iterations).

- [ ] **Step 4: Final senior-tier holistic code review**

Stage 7.5's lesson: per-task Haiku reviewers consistently miss whole-stage issues. Before tagging, dispatch ONE more review covering the entire branch:

```
The reviewer should `git log --oneline main..HEAD` and inspect the cumulative
diff. Focus areas: (a) cross-task coherence — did per-task fixes regress
earlier work; (b) the `window.rs` mouse routing — is shift-override correctly
wired; (c) selection state transitions on edge cases (selection during a
mouse-mode toggle mid-drag, drag past the screen edge, drag in scrollback);
(d) clipboard error paths — does a clipboard failure crash anything; (e) the
keymap modifier matching — false positives on Ctrl+Alt+T or Ctrl+Shift+Alt+T;
(f) test-count drift; (g) any lingering TODO / FIXME / TBD strings.
```

Subagent dispatch: use `general-purpose` with the `sonnet` model. Treat the
review's output as advisory unless flagged Critical or Important. If anything substantive surfaces, fix before tagging.

- [ ] **Step 5: Manual smoke walkthrough**

Walk the Stage 8 section of `docs/TESTING.md` (Step 1 above). Brian will exercise this on host via VNC.

- [ ] **Step 6: Commit + tag**

```bash
cd /path/to/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 8 manual smoke checklist"
git tag -a stage8-clipboard-keyboard-complete \
  -m "clipboard + keyboard shortcuts + selection complete (Stage 8 of v0.1)"
git tag --list | grep stage8
```

- [ ] **Step 7: Surface to user**

Report:
- Number of new commits on this stage (~9 implementation + 1 docs = ~10).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 9 (TOML config + scrollback + selection color customization) as the next plan.

---

## Spec coverage check

Mapping Stage 8 spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| `keymap::Shortcut` enum + `match_shortcut` | Task 1 |
| `mouse_encoder` SGR + X10 encoding | Task 2 |
| `Clipboard` wrapper over `arboard` | Task 3 |
| `SelectionTracker` state machine + multi-click + Word/Line snap | Task 4 |
| `PtySession::restart` + `App::restart_active`/`cycle_active` | Task 5 |
| `window.rs` keyboard shortcut dispatch + copy/paste/restart | Task 6 |
| `window.rs` mouse routing + mouse-mode passthrough + Shift override | Task 7 |
| Selection rect rendering in `Renderer::render` | Task 8 |
| Per-tab selection persistence | Task 5 (field on PtySession) + Task 7 (resize / typing clears) |
| Bracketed-paste wrapping | Task 6 (`handle_paste`) |
| Smoke checklist + tag | Task 9 |

**Out of scope for Stage 8 (deferred):**
- PRIMARY clipboard / middle-click paste — Stage 9
- Right-click context menu — Stage 9 / 10
- Block (column) selection — Stage 9
- Configurable shortcuts (TOML) — Stage 9
- Selection in scrollback — Stage 10
- Selection that anchors to grid content — open-ended

## Self-review

- **Spec coverage:** every Stage 8 spec requirement maps to a task (table above).
- **Placeholder scan:** no `TBD`/`TODO`/`implement later` patterns. Each step has actual code or commands.
- **Type consistency check:**
  - `Shortcut` enum defined in Task 1, consumed in Task 6 (`handle_shortcut`).
  - `Button` enum defined in Task 2, consumed in Task 7 (mouse routing).
  - `Clipboard::copy` / `paste` defined in Task 3, consumed in Task 6.
  - `SelectionTracker::mouse_down/drag/up/clear/cells/text/current/is_dragging` defined in Task 4, consumed in Task 5 (field on `PtySession`), Task 7 (mouse routing), Task 8 (rect building).
  - `Selection`, `SelectionMode` defined in Task 4 — public types used by Task 8 indirectly via `tracker.cells`.
  - `PtySession::restart` defined in Task 5, called by `App::restart_active` (Task 5) and `Window::handle_shortcut::RestartTab` (Task 6).
  - `App::cycle_active(direction: i32)` signature defined in Task 5, called by Task 6.
  - `build_selection_rects` defined in Task 8.
  - `pixel_to_grid_point` defined in Task 7, used by both `MouseInput` and `CursorMoved` arms.
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** unchanged. All selection/clipboard state on the main thread.
- **Test count tracking:** Stage 7.5 ends at 135 default + 12 ignored. Stage 8 adds:
  - keymap (Task 1): 17 default
  - mouse_encoder (Task 2): 8 default
  - clipboard (Task 3): 1 ignored
  - selection (Task 4): 14 default
  - session/app (Task 5): 2 default (restart + cycle_active; cycle_active test may be ignored on slow runners)
  - rendering (Task 8): 1 ignored
  - **Final: ~176 default + ~14 ignored.**

## Notable plan risks

1. **`AiStateTracker::config()` accessor may not exist.** Task 5 adds it if needed; failing that, falls back to `TrackerConfig::default()`. Either way, restart works.
2. **`App::push_tab`** — verified absent. Task 6's NewTab handler routes through `App::new_tab(argv)` instead, which spawns + appends + sets active in one call. Done.
3. **alacritty_terminal `Term::grid()[Point]` indexing** — Task 4's `text()` method assumes `&grid[Point]` works. If it requires `&grid[Line(n)][Column(m)]`, adapt the iteration.
4. **`TermSize` in tests may be gated** — Task 4's tests use it; if it's `#[cfg(test)]`-private to `alacritty_terminal`, the test module needs a local `Dimensions` impl. Plan provides the fallback.
5. **winit 0.30 ModifiersState API** — `modifiers.state()` accessor was correct as of writing; if winit changed, adapt. The `control_key()`/`shift_key()`/`alt_key()`/`super_key()` methods on `ModifiersState` are stable.
6. **Selection rect coordinate sign** — `Line(i32)` can be negative for scrollback. Task 8's `filter_map` rejects negative-line points so the renderer doesn't generate negative-y rects. If selection ever extends into scrollback (Stage 10+), the filter widens.
7. **arboard threading on Wayland** — arboard internally polls the display server. On Wayland with strict thread affinity (some compositors), repeated copy/paste may stutter. If smoke shows this, Stage 9 can spawn a dedicated clipboard thread.
8. **Multi-click test isolation** — `Instant::now()` is non-deterministic. Tests that depend on the 500ms window pass an explicit `Instant` parameter to `mouse_down`; the production callers pass `Instant::now()`. The signature was set up for this in Task 4.

These risks are addressed by senior pre-execution review of this plan and the Stage 8 manual smoke walkthrough before merge.
