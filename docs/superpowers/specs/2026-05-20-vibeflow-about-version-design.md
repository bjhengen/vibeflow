# vibeflow About / Version — design spec

**Date:** 2026-05-20
**Status:** Approved (brainstorm), pending implementation plan
**Branch base:** `main` @ `9d6dea1` (v0.1.1 finale + npm-version-fix on main)
**Target release:** v0.1.2 (or whenever the next small-fix bundle ships)

A small, self-contained UI feature: surface vibeflow's version to users via (a) a CLI flag and (b) an "About vibeflow" item in the existing right-click context menu that opens a centred modal overlay panel. Same overlay code serves both menu and (potentially-future) keyboard-shortcut triggers.

> Code references (file:line) below were accurate at spec time on `main`
> @ `9d6dea1`; verify against current source before editing.

---

## 1. Scope & non-goals

**In scope (v0.1.2 candidate):**

1. **CLI `--version` / `-V` flag** — print `vibeflow <version>` to stdout, exit 0. Recognised before any GUI/winit initialisation so it works headless (`vibeflow --version` over SSH, in a Dockerfile build, etc.). Currently the flag falls through to GUI launch — surprising and a Codex-like ship-quality gap.
2. **Right-click menu item "About vibeflow"** in `grid_menu()` (the right-click-on-terminal-grid menu in `render/context_menu.rs`). Appended at the bottom after an existing separator. Opens the overlay.
3. **About overlay panel** — a small centred modal showing 5 lines: name+version, gap, tagline, license · repo URL, dismissal hint. Opened by the menu item. Dismissed by ESC, click anywhere (outside or on the panel), or any other key.

**Out of scope (with rationale):**

- **Keyboard shortcut to open About** — not requested; can add later with one `Shortcut::ShowAbout` line in `keymap.rs` and a single match arm in `window.rs` re-using the same overlay state.
- **Git SHA / build metadata** — would require a `build.rs` and a `vergen`-style env var. Not asked for; adds plumbing. Easy to add later.
- **Submenu structure** (e.g. About → Version, About → License) — vibeflow's context menus are flat by design; submenus are a non-trivial UI investment for marginal gain.
- **Clickable URL that opens a browser** — would require platform-specific `xdg-open` invocation, error paths, and a hover-cursor change. The URL is rendered as plain text; users can copy via the existing selection mechanism or just type it. Add later if it becomes friction.
- **Rich credits / dependency list** — not asked for. The README already credits the major deps (winit, wgpu, cosmic-text, alacritty_terminal).
- **Native OS dialog** — winit doesn't offer one; would require a second window or an `mbox`-style external crate. The in-canvas overlay matches vibeflow's "everything we render is on the GPU" aesthetic.
- **Animation** on open/close — keep it instant. Animation work is a separate concern.

---

## 2. CLI `--version` / `-V`

**File:** `crates/vibeflow/src/main.rs`

The current `main()` parses args by hand in a small block (existing `--import-colors` detection at ~line 21). Add a new arm **above** the `--import-colors` check so it short-circuits cleanly before any later parsing or GUI init:

```rust
if args.iter().any(|a| a == "--version" || a == "-V") {
    println!("vibeflow {}", env!("CARGO_PKG_VERSION"));
    return Ok(());
}
```

**Semantics:**

- `env!("CARGO_PKG_VERSION")` is a compile-time string from `Cargo.toml`'s `version` field (currently `0.1.1`). No build script, no extra dep. The published artifact and the `--version` output are guaranteed to agree.
- The output is `vibeflow <version>\n` on stdout (println!'s newline), exit code 0. Convention matches `git --version`, `cargo --version`, `rustc --version`.
- Both `--version` and `-V` are recognised (the `-V` short form matches `cargo` and most clap-based CLIs).
- The check fires on **any** occurrence in the args — `vibeflow --version some-other-arg` still exits with the version. Matches `cargo --version` behaviour (cargo doesn't error on extra args after `--version`).

**Constraints:**

- Must NOT initialise winit, wgpu, or open any file. Verified by the headless CLI test below.
- Must NOT touch `~/.config/vibeflow/`. The flag is informational and must work on first-run installs with no config.

---

## 3. Menu integration

**File:** `crates/vibeflow/src/render/context_menu.rs`

Two changes:

1. **`MenuAction` enum** — add a `ShowAbout` variant alongside existing variants (e.g. `Copy`, `Paste`, `NewTab`, `PasteFromPrimary`, …). The variant carries no data — opening the About panel is a constant action.

2. **`grid_menu()` function** — append the new item at the bottom (after the existing items) preceded by a separator:

   ```rust
   items.push(MenuItem::separator());
   items.push(MenuItem {
       label: "About vibeflow".to_string(),
       action: MenuAction::ShowAbout,
       enabled: true,
   });
   ```

   The `tab_menu()` (right-click on a tab) does NOT get this item — About is an application-level concept, not a per-tab one; it belongs in the terminal-grid context menu.

**File:** `crates/vibeflow/src/window.rs`

The existing `MenuAction` dispatch match (where `MenuAction::Copy`, `Paste`, etc. are routed) gains one arm:

```rust
MenuAction::ShowAbout => {
    self.about_open = true;
    // Close the context menu (mutex with overlay — see §4 "Invariants").
    self.context_menu = None;
}
```

`self.about_open: bool` is a new field on `WindowApp`, initialised to `false` in `WindowApp::new` (or its equivalent constructor) — same shape as the existing `rename_state: Option<RenameInputState>` field.

---

## 4. About overlay panel

**New module:** `crates/vibeflow/src/render/about.rs`

Mirrors the structure of `render/context_menu.rs`: layout + render + constants in the module; input wiring and state lives in `window.rs`. Doc comment cross-references that pattern.

### 4.1 Content (compile-time constants)

```rust
const TAGLINE: &str = "GPU-accelerated Linux terminal that knows when your AI tool is waiting on you.";
const LICENSE: &str = "Dual-licensed: MIT OR Apache-2.0";
const REPO_URL: &str = "https://github.com/bjhengen/vibeflow";

/// Five lines, ordered top→bottom. Centred horizontally inside the panel;
/// vertical layout stacks them with even spacing inside the inner-padding box.
pub fn about_lines() -> [String; 5] {
    [
        format!("vibeflow {}", env!("CARGO_PKG_VERSION")),
        String::new(),  // intentional visual gap — keeps the version visually prominent
        TAGLINE.to_string(),
        format!("{LICENSE}  ·  {REPO_URL}"),
        "Press ESC, click outside, or click the panel to close".to_string(),
    ]
}
```

The five-line shape is fixed (a `[String; 5]` not a `Vec<String>`) so the panel-height math is constant. Tests pin both the count and the per-line content invariants.

### 4.2 Layout

```rust
/// Returns `(x, y, w, h)` in logical pixels for the centred panel.
/// `window_size` is the inner window size from winit.
pub fn panel_rect(window_size: (u32, u32)) -> (f32, f32, f32, f32) { ... }
```

- **Default panel size:** `560 × 200` logical pixels — wide enough for the URL line at the default font, tall enough for 5 lines + 16 px inner padding top and bottom.
- **Small-window clamp:** if `window_w < 600` or `window_h < 240`, the panel clamps to `window_w - 40` and `window_h - 40` (20 px margin all sides). For very small windows (< 200×120) the panel matches the window with 8 px margin. Lower-bounded so the panel is never zero-sized.
- **Centering:** `x = (window_w - w) / 2.0`, `y = (window_h - h) / 2.0`.

Unit-testable as a pure function — see §6.

### 4.3 Render

`pub fn render_about(...)` is called at the END of the frame in `render/mod.rs`, after cells, tab bar, and context menu — so the panel sits on top of everything.

Two visual layers:

1. **Backdrop dim** — a full-window translucent quad over the entire viewport, `rgba(0, 0, 0, 0.5)`. Visually focuses attention on the panel and signals "modal" (blocks visual access to the terminal cells beneath without hiding them).
2. **Panel body** — opaque rectangle at the rect from §4.2, filled with the **active theme's background colour** (`active_theme_colors[NamedColor::Background]` if present, else the `CLEAR_COLOR` fallback — reuses the helper added in the post-Stage-13 polish for the wgpu clear colour). A 2 px border in the theme's **foreground** colour (`Colors[NamedColor::Foreground]` or fallback) outlines the panel against the dim. Inside, the 5 lines from §4.1 are rendered through the existing cosmic-text + glyph-atlas pipeline (the same path the terminal cells use), text colour = theme foreground.

**Text layout:**

- Lines are horizontally centred within the panel's inner-padding box.
- Vertical positions: `inner_top + i * line_height` for line `i` ∈ {0..5}, where `inner_top = panel_y + padding_top` and `line_height = (inner_h) / 5.0` (even distribution). The empty second line is naturally absorbed as visual whitespace.

No new render pass is introduced. The backdrop and panel-body quads go through the existing quad pipeline; the text goes through the existing text pipeline. Adding the overlay is bookkeeping (extra draw calls per frame when `about_open == true`), not architecture.

### 4.4 Dismissal & input capture (wired in `window.rs`)

When `about_open == true`, **all** input is captured by the overlay before reaching the terminal or the tab bar:

- **ESC key (Pressed)** → `self.about_open = false; return;` (do NOT pass ESC bytes to the PTY).
- **Any mouse-button Pressed** (LMB/MMB/RMB, anywhere in the window — on or off the panel) → `self.about_open = false;`. The panel itself is click-to-close; the rule is uniform "any click closes." Selection drags, tab-bar clicks, etc. are all swallowed for the duration the panel is open. After close, the same gesture does NOT re-fire on the underlying surface — the click event is consumed by the dismissal.
- **Any other key (Pressed)** → swallowed (don't send bytes to PTY), the panel stays open. This gives "modal" feel: keystrokes don't accidentally drive the underlying shell. The user can ESC to dismiss, then resume typing.
- **Mouse motion** → ignored (no hover effects on the panel for v1).
- **Window resize / focus events** → handled normally (the panel re-centres on the next frame via `panel_rect()`).

### 4.5 Invariants

- **At most one full-window overlay open at a time.** Opening About closes the context menu (the menu's drop-down dismisses). Opening the context menu when About is open is structurally prevented (any click while About is open closes About first; the menu doesn't open from that same click).
- **`about_open == true` blocks renaming-overlay activation** (a Pressed key would also close About first; no rename can start while About is shown). Existing rename overlay logic is unchanged.
- **No PTY writes occur while `about_open == true`** — verified by the integration test in §6.

---

## 5. File structure (what each new/modified file does)

| Path | New? | Purpose |
|---|---|---|
| `crates/vibeflow/src/render/about.rs` | NEW | About-panel content constants, `about_lines()`, `panel_rect()`, `render_about()`. Unit-tested. |
| `crates/vibeflow/src/render/mod.rs` | MODIFY | Declare `pub mod about;`. Call `render::about::render_about(...)` at the END of the frame when `about_open`. |
| `crates/vibeflow/src/render/context_menu.rs` | MODIFY | `MenuAction::ShowAbout` variant; one new `MenuItem` in `grid_menu()`. |
| `crates/vibeflow/src/window.rs` | MODIFY | `about_open: bool` field; `MenuAction::ShowAbout` dispatch arm; input-capture branches in keyboard / mouse handlers when `about_open`. |
| `crates/vibeflow/src/main.rs` | MODIFY | `--version` / `-V` arm in arg parser. |
| `crates/vibeflow/tests/cli_version.rs` | NEW | Headless integration test for `--version` / `-V`. |

**NOT touched:** `keymap.rs` (no new shortcut in scope); `config/mod.rs` (no new config keys — the panel content is compile-time); themes/colors (the panel reuses the active theme).

---

## 6. Tests

### Unit (in `about.rs`)

- `about_lines_has_five_lines` — `assert_eq!(about_lines().len(), 5)`. Pins the layout-math invariant.
- `about_lines_first_line_starts_with_vibeflow_and_includes_version` — uses `env!("CARGO_PKG_VERSION")` so the test self-updates on version bumps.
- `about_lines_repo_url_is_canonical_github` — pins `https://github.com/bjhengen/vibeflow` so a typo (`bjehengen`, missing slash, etc.) is caught.
- `panel_rect_centres_within_window_at_default_size` — 1920×1080 input → panel at `(680, 440, 560, 200)` (or whatever the centering math actually yields; assert the centre is the window centre, and `(w, h) == (560.0, 200.0)`).
- `panel_rect_clamps_in_small_window` — 400×200 input → panel at `(20, 20, 360, 160)` (20 px margin clamp on each side).
- `panel_rect_handles_tiny_window_lower_bound` — 100×60 input → panel still has positive `w` and `h` (lower bound applied, not zero or negative).

### Integration (in `window.rs`'s existing `#[cfg(test)] mod tests`)

- `show_about_action_opens_overlay_and_closes_menu` — synthesise a `MenuAction::ShowAbout` dispatch, assert `about_open == true` AND the context menu is closed.
- `escape_while_about_open_closes_overlay_and_swallows_pty_write` — pre-set `about_open = true`, feed an ESC key event, assert `about_open == false` AND the active session's PTY received NO bytes for that event.
- `mouse_click_while_about_open_closes_overlay` — pre-set `about_open = true`, feed a `MouseButton::Left` Pressed, assert `about_open == false`.
- `random_key_while_about_open_is_swallowed` — pre-set `about_open = true`, feed an `A` key press, assert `about_open` is still `true` AND no PTY bytes were sent.

### CLI integration (new `crates/vibeflow/tests/cli_version.rs`)

Spawns the built `vibeflow` binary as a subprocess (`std::process::Command::new(env!("CARGO_BIN_EXE_vibeflow"))`) with `--version` and again with `-V`. For each:

- exit code 0
- stdout equals exactly `format!("vibeflow {}\n", env!("CARGO_PKG_VERSION"))`
- stderr is empty
- the test completes in well under 1 second (a runaway GUI init would time out or hang)

(Existing `tests/emit_cli.rs` in `crates/vibeflow-protocol/` is the reference pattern for `Command::new(env!("CARGO_BIN_EXE_..."))`.)

### Manual VNC smoke (before merge)

1. Launch `./target/release/vibeflow`; right-click on the terminal grid → menu shows "About vibeflow" at the bottom.
2. Click "About vibeflow" → centred panel appears, dim backdrop visible, 5 lines readable, theme colours match.
3. Press ESC → panel closes, cursor in terminal still works, no stray characters.
4. Open About → click outside the panel → closes.
5. Open About → click on the panel itself → closes.
6. Open About → type random characters → none reach the PTY (shell prompt didn't move).
7. From a separate terminal: `./target/release/vibeflow --version` → prints `vibeflow 0.1.x` to stdout, exits immediately (no GUI window opens).
8. `./target/release/vibeflow -V` → same as `--version`.

---

## 7. Workflow

Spec → `superpowers:writing-plans` → senior pre-execution Sonnet review of the plan vs actual source (per `feedback_senior_review_plans`) → `superpowers:subagent-driven-development` (fresh implementer + spec-compliance reviewer + code-quality reviewer per task; reviewers read-only per `lesson_review_subagent_destructive`; controller runs `git status` after every task per `lesson_subagent_amend_drift`; implementer prompts include pasted-evidence requirements per `lesson_subagent_ignored_publish_guard`) → manual VNC smoke walk (per §6) → senior holistic review → merge `main` `--no-ff` → bundle for v0.1.2 (whenever next small-fix batch ships) or ship standalone.

No new `release.yml` or `ci.yml` changes — the CI packaging job from v0.1.1 already asserts the published crate's contents and will continue working unchanged. The new `cli_version.rs` integration test is automatically picked up by `cargo test --workspace --all-targets` in the existing `rust` job.

## 8. Open questions / explicit non-decisions

- **Logo / icon in the panel** — out of scope for v1. The window-icon work in Stage 14 embedded `crates/vibeflow/assets/icon.png` (256×256) and the renderer already knows how to decode it. Future iteration could draw the icon at the top of the About panel; this v1 keeps the panel text-only for minimum render-pipeline impact.
- **Larger window-size adaptive layouts** — out of scope. The fixed 560×200 panel with small-window clamp is sufficient; a multi-breakpoint adaptive layout is YAGNI for an About dialog.
- **Localisation** — vibeflow has no i18n infrastructure. All About text is English-only, hardcoded. If/when i18n lands, the constants in `about.rs` become message-catalog keys; no other change needed.
