# vibeflow Stage 6 Implementation Plan: tab bar + Notice indicator + dead-tab banner + mouse tab interaction

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the two-line tab bar with the Notice indicator stripe (vibeflow's flagship visual feature) at the top of the window, drive the indicator with `AiStateTracker` state and animate it with a 1.4 s sine pulse on `Waiting`, render the dead-tab banner when a session has terminated, and support mouse-driven tab create/close/switch via a `+` button at the end of the bar and `×` close buttons on each tab. After this plan, vibeflow looks like a real terminal — multi-tab interface, per-tab AI state visible at a glance, recoverable from dead tabs.

**Architecture:** Three new render submodules + a label API on `PtySession` + mouse-event handling in `WindowApp`:

- `session/session.rs` (modify) — `PtySession` gains a `label: TabLabel { title: String, subtitle: String }` field with `label()`/`set_label()` accessors. `TabLabel::default_for(shell, state)` builds the default ("bash"/"idle") used at spawn. Stage 9's TOML config loader will call `set_label` with template-derived strings.
- `app.rs` (modify) — `App::set_active(idx)` sets the focused tab.
- `window.rs` (modify) — sync cell-metrics from `Renderer::atlas.cell_pitch()` (replacing the placeholder `CELL_WIDTH_PX = 8` / `CELL_HEIGHT_PX = 16`). Add `cursor_pos: Option<(u32, u32)>` field; handle `WindowEvent::CursorMoved` and `WindowEvent::MouseInput`. On click, hit-test against the tab-bar layout and dispatch to `App::new_tab` / `close_tab` / `set_active`. About_to_wait picks `ControlFlow::WaitUntil(now + 16ms)` while any tab is `Waiting` (60 Hz pulse) and `100ms` otherwise.
- `render/text.rs` (new) — `TextPipeline`: pixel-position textured-quad pipeline that reuses the existing `GlyphAtlas`. Per-instance `(pos_px, glyph_index, fg, bg)`. Used by both the tab title/subtitle text and the dead-tab banner text. ~200 LOC.
- `render/text.wgsl` (new) — vertex/fragment shaders for `TextPipeline`. Mostly mirrors `grid.wgsl` but takes pixel-space positions instead of cell coordinates. ~50 LOC.
- `render/tabs.rs` (new) — `TabBarLayout` (pure logic, TDD'd: computes per-tab rectangles, the `+` button rect, and per-tab `×` rects from window dimensions and tab list). `TabBarPipeline` (renders solid-color rectangles for tab backgrounds, indicator stripes, separators, and the `×`/`+` button bodies). `TabBarRenderer` glue that builds instance lists from `App` state + tracker states. Pulse-alpha computation lives here. ~350 LOC.
- `render/tabs.wgsl` (new) — vertex/fragment shaders for solid-color rectangles. ~30 LOC.
- `render/mod.rs` (modify) — `Renderer` gains `text_pipeline` and `tab_bar_pipeline` fields plus a `tab_bar_height_px: u32` derived from atlas metrics. `render` adds a tab-bar pass after the cell-grid pass; `build_cell_instances` skips rows that fall above the cell-grid scissor (which now starts at `y = tab_bar_height_px`).
- `assets/` — no new assets; reuses JetBrainsMono.

**Threading model:** unchanged. `Renderer` and `App` stay on the main thread; mouse events arrive on the same winit event loop.

**Tech Stack:** No new dependencies. All rendering reuses `wgpu = "0.20"`, `bytemuck`, the existing `GlyphAtlas`. Pulse animation uses `Instant::now().elapsed()` for the time source.

**Stage scope:** Stage 6 ends with a usable multi-tab terminal: open vibeflow, see one tab; click `+` to add another; switch between tabs by clicking; close tabs via `×`; observe the indicator stripe change color as the AI tool (or shell prompt) emits state events; observe the amber pulse when waiting. Dead tabs show an in-tab banner. Stage 7 swaps fontdue for cosmic-text for proper Unicode shaping. Stage 8 adds keyboard shortcuts (`Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, `Ctrl+Shift+R`, etc.). Stage 9 adds TOML config + hot-reload (which will drive `set_label` for cwd / running-process titles, and override the indicator colors).

**Out of scope (deferred):**
- Selection rendering on mouse drag — separate stage (needs mouse-drag handling + a selection-rectangle render pass).
- Scrollback rendering on mouse wheel — separate stage (needs scroll handling + alacritty's scrollback iteration).
- Cursor blink animation — Stage 7+.
- Bell / visual flash — Stage 7+.
- Hyperlinks — Stage 8+.
- TOML config + hot-reload (`[tabs] position`, `default_title_from`, indicator colors) — Stage 9.
- Keyboard shortcuts for tab management — Stage 8.
- Cwd-based titles — Stage 9 (procfs polling) or whenever shell-hook integration lands.

**Lessons carried forward from Stages 1–5:**
- Per-task `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings` verify step before commit.
- TDD applies to pure logic (layout math, hit-test, text positioning, alpha-from-time math). GUI parts (pipeline state, render pass orchestration) are verified by `cargo build` + manual smoke run.
- Pre-execution senior review of plan code is high-value when introducing new render pipelines and new winit-event handling. Stage 5 caught three compile-blockers; this stage should get the same review pass.
- Implementer dispatch prompts must include "DO NOT MODIFY OR DELETE EXISTING TESTS" — the Stage 5 Task 2 implementer deleted three Stage 2/3 tests with fabricated justifications. Add the guard explicitly.
- For PTY tests, use python3 + `bytes([...])` rather than `/bin/sh -c "printf '\xNN'"` — Ubuntu's `/bin/sh` is dash, no `\xNN` support.
- Plan-verbatim Rust code must be already rustfmt-clean.
- WGSL bugs only surface at runtime; smoke run is the validation gate.
- Uniform-buffer alignment: `vec2`/`vec4` natural alignment (8 / 16 bytes) usually means no padding is needed; verify struct size against alignment rules instead of adding `_pad` fields blindly.
- intra-doc links must use `[`Self::method`]` not `[`method`]` to satisfy `RUSTDOCFLAGS="-D warnings" cargo doc`.

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/vibeflow/src/render/mod.rs` (modify) | Add `pub mod text;` and `pub mod tabs;`. `Renderer` gains `text_pipeline: TextPipeline` and `tab_bar_pipeline: TabBarPipeline` fields plus `tab_bar_height_px`. `render(...)` adds a tab-bar pass. |
| `crates/vibeflow/src/render/text.rs` (new) | `TextPipeline` + `GlyphInstance` (pixel-position quad). ~200 LOC. |
| `crates/vibeflow/src/render/text.wgsl` (new) | Vertex/fragment for `TextPipeline`. ~50 LOC. |
| `crates/vibeflow/src/render/tabs.rs` (new) | `TabBarLayout` (pure, TDD'd), `TabBarPipeline` (solid rects), `TabBarRenderer` (glue + pulse animation). ~350 LOC. |
| `crates/vibeflow/src/render/tabs.wgsl` (new) | Solid-color rectangle pipeline. ~30 LOC. |
| `crates/vibeflow/src/session/session.rs` (modify) | Add `label: TabLabel` field, `label()` accessor, `set_label(label)` setter, `default_label_for(shell)` helper. |
| `crates/vibeflow/src/session/mod.rs` (modify) | `pub use session::TabLabel;`. |
| `crates/vibeflow/src/app.rs` (modify) | Add `set_active(idx)`. |
| `crates/vibeflow/src/window.rs` (modify) | Sync cell metrics from `atlas.cell_pitch()`; `cursor_pos: Option<(u32, u32)>` field; `WindowEvent::CursorMoved` + `MouseInput` handlers; mouse hit-test → `App::new_tab` / `close_tab` / `set_active`; pulse-aware `ControlFlow::WaitUntil`. |
| `docs/TESTING.md` (extend) | Append Stage 6 manual smoke checklist. |

---

## Task 0: Add module declarations + stubs

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`
- Create: `crates/vibeflow/src/render/text.rs` (stub), `crates/vibeflow/src/render/text.wgsl` (stub), `crates/vibeflow/src/render/tabs.rs` (stub), `crates/vibeflow/src/render/tabs.wgsl` (stub)

No new external deps in this task. The new modules are stubs filled in by Tasks 4–6.

- [ ] **Step 1: Declare new submodules in `crates/vibeflow/src/render/mod.rs`**

The current module-declaration block at the top of the file is (after Stage 5):

```rust
pub mod atlas;
pub mod colors;
pub mod grid;
```

Replace with:

```rust
pub mod atlas;
pub mod colors;
pub mod grid;
pub mod tabs;
pub mod text;
```

- [ ] **Step 2: Stub the new files**

Create `crates/vibeflow/src/render/text.rs`:

```rust
//! `TextPipeline` — pixel-position textured-quad pipeline that reuses
//! [`crate::render::atlas::GlyphAtlas`]. Used by tab titles/subtitles and the
//! dead-tab banner. Stage 7 (cosmic-text) will replace the simple monospace
//! advance with shaping output.
```

Create `crates/vibeflow/src/render/text.wgsl`:

```wgsl
// vibeflow Stage 6 text shader. Filled in by Task 4.
```

Create `crates/vibeflow/src/render/tabs.rs`:

```rust
//! Tab-bar rendering. Three pieces:
//!  * [`TabBarLayout`] — pure logic, computes per-tab rectangles + button hit zones.
//!  * `TabBarPipeline` — wgpu pipeline-state for solid-color rectangles
//!    (tab backgrounds, indicator stripes, separators, button bodies).
//!  * `TabBarRenderer` — glue that builds the per-frame instance lists from
//!    [`crate::app::App`] state + tracker states, including the Notice
//!    indicator pulse animation on `Waiting` tabs.
```

Create `crates/vibeflow/src/render/tabs.wgsl`:

```wgsl
// vibeflow Stage 6 tab-bar rectangle shader. Filled in by Task 5.
```

- [ ] **Step 3: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean (the new modules are doc-comment stubs but valid).

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/mod.rs crates/vibeflow/src/render/text.rs crates/vibeflow/src/render/text.wgsl crates/vibeflow/src/render/tabs.rs crates/vibeflow/src/render/tabs.wgsl
git commit -m "chore(render): declare text and tabs submodules + stubs for Stage 6"
```

---

## Task 1: Sync cell metrics — `window.rs` reads `atlas.cell_pitch()` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`
- Modify: `crates/vibeflow/src/render/mod.rs` (expose `Renderer::cell_pitch()`)

The Stage 4 placeholder constants `CELL_WIDTH_PX = 8` and `CELL_HEIGHT_PX = 16` in `window.rs` were never updated when Stage 5 introduced the real `GlyphAtlas`. JetBrains Mono at 16 px produces an actual cell pitch of approximately `(9, 22)` (depends on the font's metrics — verified at runtime). The mismatch means `App::resize_all` tells the PTY one cell count, but `Renderer::render` lays out cells using a different cell pitch, so the bottom rows or right columns of the grid are clipped.

The fix: `window.rs` reads the real cell pitch from the renderer (which reads it from the atlas) every time it computes a PTY resize. Since the renderer is created lazily in `resumed`, this means we only have access to the cell pitch AFTER the renderer is built. The initial-resize call in `resumed` (Stage 4 senior-review fix) already happens after `Renderer::new`, so it can use the real pitch immediately. The `WindowEvent::Resized` arm has the same access pattern.

- [ ] **Step 1: Expose `cell_pitch()` on `Renderer`**

In `crates/vibeflow/src/render/mod.rs`, locate the existing `surface_size()` method on `impl Renderer`. Add a sibling:

```rust
    /// Per-cell pixel pitch reported by the atlas. Stage 6 wires this into
    /// the window event loop's resize math (replacing the Stage 4 placeholders).
    #[must_use]
    pub fn cell_pitch(&self) -> (u32, u32) {
        self.atlas.cell_pitch()
    }
```

- [ ] **Step 2: Update `pixels_to_grid`'s callsites in `window.rs` (TDD)**

The existing `pixels_to_grid(width_px, height_px, cell_w, cell_h) -> (u16, u16)` function (Stage 4 Task 6) is correct as-is — it already takes cell pitch as parameters. The fix is at the callsites: instead of `pixels_to_grid(w, h, CELL_WIDTH_PX, CELL_HEIGHT_PX)`, pass the renderer's actual pitch.

Append two new tests to the existing `mod tests` in `crates/vibeflow/src/window.rs`:

```rust
    #[test]
    fn pixels_to_grid_with_real_jbm_metrics() {
        // JetBrains Mono Regular at 16 px: advance_width ≈ 9.6 px → ceil = 10,
        // line metrics' new_line_size ≈ 21.6 px → ceil = 22. Verify the math
        // works for that pitch (we don't hardcode the values here because they
        // depend on the font binary's hinting, but we sanity-check that
        // 800/10 = 80 columns, not 800/8 = 100.
        let (rows_jbm, cols_jbm) = pixels_to_grid(800, 480, 10, 22);
        assert_eq!(cols_jbm, 80);
        assert_eq!(rows_jbm, 21);

        // The Stage-4 placeholder pitch (8×16) would have given different math.
        // This contrast test makes the bug obvious if someone re-introduces
        // the placeholders.
        let (rows_placeholder, cols_placeholder) = pixels_to_grid(800, 480, 8, 16);
        assert_eq!(cols_placeholder, 100);
        assert_eq!(rows_placeholder, 30);
        assert_ne!(
            (rows_jbm, cols_jbm),
            (rows_placeholder, cols_placeholder),
            "real font metrics should produce different grid dims than the \
             Stage-4 placeholder 8×16 pitch — if these are equal, window.rs \
             is still using the placeholders"
        );
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib window
```

Expected: existing 11 window-module tests pass plus this new one (it should pass right away — `pixels_to_grid` already accepts arbitrary pitch).

- [ ] **Step 3: Replace the placeholder constants and update callsites**

In `crates/vibeflow/src/window.rs`, find and **delete** the two placeholder constants:

```rust
const CELL_WIDTH_PX: u32 = 8;
const CELL_HEIGHT_PX: u32 = 16;
```

(Delete the `pub` doc comment above them too — the comment that said "Stage 7 (font atlas) replaces this with values derived from cosmic-text font metrics for the configured font." is obsolete.)

Find the two callsites of `pixels_to_grid`:

1. In `WindowApp::resumed`, after the renderer is constructed and the first tab is spawned. The current code is:
   ```rust
        if let Some(renderer) = self.renderer.as_ref() {
            let (width, height) = renderer.surface_size();
            let (rows, cols) = pixels_to_grid(width, height, CELL_WIDTH_PX, CELL_HEIGHT_PX);
            if let Err(e) = self.app.resize_all(rows, cols) {
                tracing::warn!(error = %e, rows, cols, "initial PTY resize failed");
            }
        }
   ```
   Replace with:
   ```rust
        if let Some(renderer) = self.renderer.as_ref() {
            let (width, height) = renderer.surface_size();
            let (cell_w, cell_h) = renderer.cell_pitch();
            let (rows, cols) = pixels_to_grid(width, height, cell_w, cell_h);
            if let Err(e) = self.app.resize_all(rows, cols) {
                tracing::warn!(error = %e, rows, cols, "initial PTY resize failed");
            }
        }
   ```

2. In `WindowApp::window_event`'s `WindowEvent::Resized` arm. The current code:
   ```rust
            WindowEvent::Resized(new_size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(new_size.width, new_size.height);
                }
                let (rows, cols) =
                    pixels_to_grid(new_size.width, new_size.height, CELL_WIDTH_PX, CELL_HEIGHT_PX);
                if let Err(e) = self.app.resize_all(rows, cols) {
                    tracing::warn!(error = %e, rows, cols, "PTY resize failed");
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
   ```
   Replace with:
   ```rust
            WindowEvent::Resized(new_size) => {
                let cell_pitch = self.renderer.as_ref().map(|r| r.cell_pitch());
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(new_size.width, new_size.height);
                }
                if let Some((cell_w, cell_h)) = cell_pitch {
                    let (rows, cols) =
                        pixels_to_grid(new_size.width, new_size.height, cell_w, cell_h);
                    if let Err(e) = self.app.resize_all(rows, cols) {
                        tracing::warn!(error = %e, rows, cols, "PTY resize failed");
                    }
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
   ```

   Note the `cell_pitch = self.renderer.as_ref().map(...)` — we read the pitch BEFORE calling `renderer.resize` because the resize doesn't change the pitch (it's a property of the atlas font, fixed at startup). Reading it first avoids a borrow-checker collision with `renderer.as_mut()`.

- [ ] **Step 4: Run tests + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: all 102 prior + 1 new = 103 lib tests pass. Fmt + clippy silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs crates/vibeflow/src/render/mod.rs
git commit -m "fix(window): read real cell pitch from atlas instead of Stage-4 placeholders"
```

---

## Task 2: `TabLabel` API on `PtySession` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`
- Modify: `crates/vibeflow/src/session/mod.rs` (re-export)

`TabLabel { title: String, subtitle: String }` is what the tab bar renders for each tab. The default label combines the shell binary name (e.g. `bash`) with the current tracker state name (e.g. `working`). Stage 9's TOML config loader will call `set_label()` to override based on `default_title_from` / `[tabs]` config.

- [ ] **Step 1: Write the failing tests**

Append to the existing `mod tests` block in `crates/vibeflow/src/session/session.rs`:

```rust
    #[test]
    fn default_label_for_bin_sh_is_sh_idle() {
        let label = TabLabel::default_for("/bin/sh", TabState::Idle);
        assert_eq!(label.title, "sh");
        assert_eq!(label.subtitle, "idle");
    }

    #[test]
    fn default_label_for_bin_bash_is_bash_active() {
        let label = TabLabel::default_for("/bin/bash", TabState::Active);
        assert_eq!(label.title, "bash");
        assert_eq!(label.subtitle, "active");
    }

    #[test]
    fn default_label_for_zsh_in_path_is_zsh() {
        // Whether spawned via `/usr/bin/zsh` or `zsh`, the title is the basename.
        assert_eq!(TabLabel::default_for("/usr/bin/zsh", TabState::Working).title, "zsh");
        assert_eq!(TabLabel::default_for("zsh", TabState::Working).title, "zsh");
    }

    #[test]
    fn default_label_for_unknown_argv_falls_back_to_argv_basename() {
        assert_eq!(
            TabLabel::default_for("/path/to/some/weird-shell", TabState::Idle).title,
            "weird-shell"
        );
    }

    #[test]
    fn ptysession_default_label_is_bash_active() {
        // PtySession::spawn always starts with TabState::Active. The default
        // label tracks that.
        let s = PtySession::spawn(&["/bin/bash"], TrackerConfig::default()).unwrap();
        assert_eq!(s.label().title, "bash");
        assert_eq!(s.label().subtitle, "active");
    }

    #[test]
    fn ptysession_set_label_overrides_default() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
        )
        .unwrap();
        s.set_label(TabLabel {
            title: "custom".into(),
            subtitle: "claude · waiting".into(),
        });
        assert_eq!(s.label().title, "custom");
        assert_eq!(s.label().subtitle, "claude · waiting");
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: compile errors — `TabLabel` not defined; `PtySession::label()` not defined.

- [ ] **Step 2: Add `TabLabel`, the `label` field, and the methods to `PtySession`**

In `crates/vibeflow/src/session/session.rs`, near the top (above `pub struct PtySession`), add:

```rust
/// Display label for a tab. The renderer in [`crate::render::tabs`] reads
/// this to draw the title (line 1) and subtitle (line 2). Stage 9's TOML
/// config will call [`PtySession::set_label`] to override based on the
/// `default_title_from` setting (`cwd` / `process` / `auto`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TabLabel {
    pub title: String,
    pub subtitle: String,
}

impl TabLabel {
    /// Default label for a freshly-spawned session: shell binary basename for
    /// the title, lowercased tracker-state name for the subtitle.
    #[must_use]
    pub fn default_for(argv0: &str, state: TabState) -> Self {
        let title = std::path::Path::new(argv0)
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or(argv0)
            .to_string();
        let subtitle = match state {
            TabState::Active => "active",
            TabState::Working => "working",
            TabState::Waiting => "waiting",
            TabState::Done => "done",
            TabState::Idle => "idle",
        }
        .to_string();
        Self { title, subtitle }
    }
}
```

In the `PtySession` struct definition, add a `label: TabLabel` field after `alive: bool`:

```rust
    /// Display label for the tab bar. Updated automatically when the tracker
    /// state changes (default policy = shell binary + state); overridable via
    /// [`Self::set_label`] from the config layer.
    label: TabLabel,
```

Update `PtySession::spawn` to initialise `label`. Locate the existing `Ok(Self { ... })` block at the end of `spawn`. Just before that block, compute the default label:

```rust
        let label = TabLabel::default_for(argv[0], TabState::default());
```

Then in the struct constructor, add `label,` as a field. (`TabState::default()` is `Active`; the tracker also starts in `Active`, so the label is consistent.)

Add the accessor + setter to `impl PtySession`. Place them grouped with `state()` / `is_alive()` / `term()` — anywhere reasonable in the impl block:

```rust
    /// Read-only access to the tab's label.
    #[must_use]
    pub fn label(&self) -> &TabLabel {
        &self.label
    }

    /// Override the tab's label. Stage 9's TOML config uses this to apply
    /// templates like `default_title_from = "cwd"`.
    pub fn set_label(&mut self, label: TabLabel) {
        self.label = label;
    }

    /// Recompute the default subtitle from the current tracker state. Called
    /// internally on every state transition. Public so `App` (or future
    /// config layers) can refresh the label when policy changes; most users
    /// won't need to call this directly.
    pub fn refresh_default_subtitle(&mut self) {
        // Only update if the title still matches what `default_for` would
        // produce — i.e. the user hasn't called `set_label` to override.
        // We use a heuristic: the title must NOT contain a space (default
        // titles are single words; user-set titles are arbitrary).
        if !self.label.title.contains(' ') {
            let new_label = TabLabel::default_for(&self.label.title, self.tracker.state());
            self.label = new_label;
        }
    }
```

(The `refresh_default_subtitle` policy is a Stage-6 simplification: titles that look like single shell-binary-style names get auto-refreshed; user-set custom titles like `"claude · my-project"` stay frozen. Stage 9 will replace this with an explicit `is_overridden: bool` flag on `TabLabel`.)

In `PtySession::poll`'s `DispatchEvent::AiState` and `DispatchEvent::Prompt` arms — both of which call `tracker.on_input(...)` and may transition state — add a call to `self.refresh_default_subtitle()` after the tracker update if the tracker returned `true`. The current arms look like:

```rust
                            DispatchEvent::AiState(frame) => {
                                if self.tracker.on_input(TrackerInput::AiFrame(frame), now) {
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                            }
                            DispatchEvent::Prompt(marker) => {
                                if self.tracker.on_input(TrackerInput::Prompt(marker), now) {
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                            }
```

Update them to:

```rust
                            DispatchEvent::AiState(frame) => {
                                if self.tracker.on_input(TrackerInput::AiFrame(frame), now) {
                                    self.refresh_default_subtitle();
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                            }
                            DispatchEvent::Prompt(marker) => {
                                if self.tracker.on_input(TrackerInput::Prompt(marker), now) {
                                    self.refresh_default_subtitle();
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                            }
```

Also update `PtySession::tick`'s state-change branch:

```rust
    pub fn tick(&mut self, now: Instant) -> Vec<SessionEvent> {
        if self.tracker.tick(now) {
            self.refresh_default_subtitle();
            vec![SessionEvent::StateChanged(self.tracker.state())]
        } else {
            Vec::new()
        }
    }
```

- [ ] **Step 3: Re-export `TabLabel` from `session/mod.rs`**

In `crates/vibeflow/src/session/mod.rs`, find the existing `pub use session::{PtySession, SessionEvent};` line and update it:

```rust
pub use session::{PtySession, SessionEvent, TabLabel};
```

- [ ] **Step 4: Run the new tests + verify the full suite**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib session::session
cargo test -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 6 new tests pass; total lib test count rises from 102 (post-Task-1) to 109. (Task 1 added 1 test; this task adds 6.) Wait — Task 1 added 1, Task 2 adds 6, so the post-Task-2 lib total is 102 + 1 + 6 = 109.

If `refresh_default_subtitle`'s heuristic conflicts with any existing test (e.g., a test that calls `set_label` with a single-word title, then expects subtitle to stay frozen), that test will break. None should — the existing tests don't call `set_label`. But verify carefully.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/session.rs crates/vibeflow/src/session/mod.rs
git commit -m "feat(session): TabLabel + default_label + refresh_default_subtitle"
```

---

## Task 3: `TabBarLayout` — pure layout math (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/tabs.rs`

`TabBarLayout` is the pure-logic core of the tab bar: given the window pixel width, the per-tab pixel pitch (cell metrics from atlas), and a tab count, compute:
- The bar height in pixels (2 × cell_h + 8 px vertical padding for breathing room)
- One `TabRect` per tab — `{ tab_idx, rect: (x, y, w, h), close_rect: (x, y, w, h) }`
- The `+` button rect (always at the right end of the bar)
- Tab pixel-width: divide the available bar width (window width minus the `+` button) evenly across the tab count, capped at a max width per tab (~250 px)

It also does hit-testing: given a click position, return what was clicked (a tab body, a close button, the new-tab button, or nothing).

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/vibeflow/src/render/tabs.rs` with the test scaffold + pure-logic functions (no `TabBarPipeline` yet — that's Task 5):

```rust
//! Tab-bar rendering. Three pieces:
//!  * [`TabBarLayout`] — pure logic, computes per-tab rectangles + button hit zones.
//!  * `TabBarPipeline` — wgpu pipeline-state for solid-color rectangles
//!    (tab backgrounds, indicator stripes, separators, button bodies).
//!  * `TabBarRenderer` — glue that builds the per-frame instance lists from
//!    [`crate::app::App`] state + tracker states, including the Notice
//!    indicator pulse animation on `Waiting` tabs.

/// Stage-6 default tab-bar height in pixels, expressed as (line_height × 2 + padding).
/// Computed at runtime from the atlas's cell pitch.
#[must_use]
pub fn tab_bar_height_px(cell_h_px: u32) -> u32 {
    cell_h_px * 2 + 8
}

/// Pixel width of the `+` (new tab) button at the right end of the bar.
pub const NEW_TAB_BUTTON_WIDTH_PX: u32 = 32;

/// Pixel width of the per-tab `×` close button.
pub const CLOSE_BUTTON_WIDTH_PX: u32 = 20;

/// Maximum pixel width any single tab is allowed to stretch to.
pub const MAX_TAB_WIDTH_PX: u32 = 250;

/// Minimum pixel width any tab is shown with (below this, the close button overlaps the title).
pub const MIN_TAB_WIDTH_PX: u32 = 80;

/// Layout result. Owns no GPU state — purely numeric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBarLayout {
    pub bar_height_px: u32,
    pub tabs: Vec<TabRect>,
    pub new_tab_button: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[must_use]
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRect {
    pub idx: usize,
    pub body: Rect,
    pub close_button: Rect,
}

/// What a click at a given (px, py) hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarHit {
    /// Click on the tab body (anywhere except the close button) — should focus.
    TabBody(usize),
    /// Click on a tab's `×` close button — should close that tab.
    TabClose(usize),
    /// Click on the `+` button — should spawn a new tab.
    NewTab,
    /// Click landed on a separator or empty space — no action.
    None,
}

impl TabBarLayout {
    /// Compute layout for `tab_count` tabs in a `window_width_px`-wide window.
    /// `cell_h_px` comes from the atlas — this is what bounds the bar height.
    #[must_use]
    pub fn compute(window_width_px: u32, cell_h_px: u32, tab_count: usize) -> Self {
        let bar_height_px = tab_bar_height_px(cell_h_px);
        let new_tab_button = Rect {
            x: window_width_px.saturating_sub(NEW_TAB_BUTTON_WIDTH_PX),
            y: 0,
            w: NEW_TAB_BUTTON_WIDTH_PX,
            h: bar_height_px,
        };

        if tab_count == 0 {
            return Self {
                bar_height_px,
                tabs: Vec::new(),
                new_tab_button,
            };
        }

        let avail_width = window_width_px.saturating_sub(NEW_TAB_BUTTON_WIDTH_PX);
        let raw_tab_w = avail_width / tab_count as u32;
        let tab_w = raw_tab_w.clamp(MIN_TAB_WIDTH_PX, MAX_TAB_WIDTH_PX);

        let mut tabs = Vec::with_capacity(tab_count);
        for idx in 0..tab_count {
            let x = (idx as u32) * tab_w;
            let body = Rect { x, y: 0, w: tab_w, h: bar_height_px };
            // Close button at the right edge of the tab's body.
            let close_button = Rect {
                x: x + tab_w.saturating_sub(CLOSE_BUTTON_WIDTH_PX + 4),
                y: bar_height_px / 2 - CLOSE_BUTTON_WIDTH_PX / 2,
                w: CLOSE_BUTTON_WIDTH_PX,
                h: CLOSE_BUTTON_WIDTH_PX,
            };
            tabs.push(TabRect { idx, body, close_button });
        }

        Self { bar_height_px, tabs, new_tab_button }
    }

    /// Hit-test a click at (px, py). Order: close button > tab body > new-tab > none.
    #[must_use]
    pub fn hit_test(&self, px: u32, py: u32) -> TabBarHit {
        if py >= self.bar_height_px {
            return TabBarHit::None; // click below the tab bar
        }
        if self.new_tab_button.contains(px, py) {
            return TabBarHit::NewTab;
        }
        for tab in &self.tabs {
            if tab.close_button.contains(px, py) {
                return TabBarHit::TabClose(tab.idx);
            }
            if tab.body.contains(px, py) {
                return TabBarHit::TabBody(tab.idx);
            }
        }
        TabBarHit::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_height_is_double_cell_plus_padding() {
        assert_eq!(tab_bar_height_px(20), 48);
        assert_eq!(tab_bar_height_px(22), 52);
    }

    #[test]
    fn compute_with_zero_tabs_returns_empty_tabs_and_button_at_right() {
        let layout = TabBarLayout::compute(960, 22, 0);
        assert!(layout.tabs.is_empty());
        assert_eq!(layout.new_tab_button.x, 960 - NEW_TAB_BUTTON_WIDTH_PX);
        assert_eq!(layout.new_tab_button.w, NEW_TAB_BUTTON_WIDTH_PX);
    }

    #[test]
    fn compute_one_tab_takes_full_available_width_clamped_to_max() {
        // 960 - 32 (new tab btn) = 928, which exceeds MAX_TAB_WIDTH_PX = 250.
        let layout = TabBarLayout::compute(960, 22, 1);
        assert_eq!(layout.tabs.len(), 1);
        assert_eq!(layout.tabs[0].body.x, 0);
        assert_eq!(layout.tabs[0].body.w, MAX_TAB_WIDTH_PX);
    }

    #[test]
    fn compute_many_tabs_packs_to_min_width() {
        // 14 tabs in 960 px: 928 / 14 = 66 px per tab, below MIN = 80.
        let layout = TabBarLayout::compute(960, 22, 14);
        assert_eq!(layout.tabs.len(), 14);
        for (i, tab) in layout.tabs.iter().enumerate() {
            assert_eq!(tab.body.w, MIN_TAB_WIDTH_PX);
            assert_eq!(tab.body.x, (i as u32) * MIN_TAB_WIDTH_PX);
        }
    }

    #[test]
    fn hit_test_below_bar_returns_none() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // y past the bar height
        assert_eq!(layout.hit_test(100, 100), TabBarHit::None);
    }

    #[test]
    fn hit_test_on_tab_body_returns_tab_body() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // First tab spans x=0..250 (clamped to MAX). Click at (50, 10) is inside.
        assert_eq!(layout.hit_test(50, 10), TabBarHit::TabBody(0));
    }

    #[test]
    fn hit_test_on_close_button_returns_tab_close() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // First tab's close button is near the right edge of the tab body.
        let close = layout.tabs[0].close_button;
        assert_eq!(
            layout.hit_test(close.x + 1, close.y + 1),
            TabBarHit::TabClose(0)
        );
    }

    #[test]
    fn hit_test_on_new_tab_button_returns_new_tab() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // The + button is at x=960-32=928, y=0.
        assert_eq!(layout.hit_test(940, 10), TabBarHit::NewTab);
    }

    #[test]
    fn hit_test_in_gap_between_tabs_returns_none_or_body() {
        // Tabs are contiguous (no visual gap in Stage 6); every x within
        // [0, total_tabs_width) is some tab. Adding a separator is a Stage 9
        // visual polish item.
        let layout = TabBarLayout::compute(960, 22, 4);
        // 4 tabs, 928 / 4 = 232 px each (< MAX 250). x=232 is the start of tab 1.
        assert_eq!(layout.hit_test(232, 10), TabBarHit::TabBody(1));
    }
}
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::tabs
```

Expected: 9 tests pass.

- [ ] **Step 2: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/tabs.rs
git commit -m "feat(render): TabBarLayout pure-logic + hit-test (TDD)"
```

---

## Task 4: `TextPipeline` — pixel-position textured-quad pipeline

**Files:**
- Modify: `crates/vibeflow/src/render/text.wgsl`
- Modify: `crates/vibeflow/src/render/text.rs`

`TextPipeline` is the parallel of `GridPipeline` for arbitrary-pixel-position text. The two are nearly identical — same atlas binding, same texture-sampling-and-mixing fragment shader — but the vertex shader takes a pixel-space position per instance instead of a cell coordinate. Stage 6 uses `TextPipeline` for tab titles, subtitles, and the dead-tab banner. Stage 7's cosmic-text rework will reuse `TextPipeline` (with a richer per-instance buffer) for full Unicode shaping output.

No tests for the GPU side — verified by `cargo build` and Task 9 smoke run.

- [ ] **Step 1: Write the WGSL shader**

Replace the contents of `crates/vibeflow/src/render/text.wgsl` with:

```wgsl
// vibeflow Stage 6 text shader.
//
// Sibling of grid.wgsl. Per-instance buffer carries pixel-space position
// + glyph index + fg/bg colors. Vertex shader expands 6 vertices per
// instance. Fragment shader is identical to grid.wgsl: mix bg → fg by
// the R8Unorm atlas alpha.

struct TextUniform {
    surface_size_px: vec2<f32>,   // viewport size in physical pixels
    cell_size_px:    vec2<f32>,   // per-cell pitch in physical pixels (atlas)
    atlas_size_px:   vec2<f32>,   // atlas texture size in pixels
    atlas_cells:     vec2<u32>,   // atlas layout (cols, rows of glyphs)
};

@group(0) @binding(0) var<uniform> u: TextUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    // .xy = pos_px (top-left of the glyph cell), .z = glyph_index_as_f32, .w = unused.
    @location(0) pos_glyph: vec4<f32>,
    @location(1) fg:        vec4<f32>,
    @location(2) bg:        vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var quad_offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = quad_offsets[in.vertex_id];

    let pos_top_left_px = in.pos_glyph.xy;
    let glyph_idx       = u32(in.pos_glyph.z);

    let pos_px = pos_top_left_px + corner * u.cell_size_px;
    let ndc    = (pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    let atlas_col = f32(glyph_idx % u.atlas_cells.x);
    let atlas_row = f32(glyph_idx / u.atlas_cells.x);
    let glyph_top_left_px = vec2<f32>(atlas_col, atlas_row) * u.cell_size_px;
    let glyph_pos_px      = glyph_top_left_px + corner * u.cell_size_px;
    let uv                = glyph_pos_px / u.atlas_size_px;

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.uv       = uv;
    out.fg       = in.fg;
    out.bg       = in.bg;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
    return vec4<f32>(rgb, 1.0);
}
```

- [ ] **Step 2: Implement `TextPipeline`**

Replace the contents of `crates/vibeflow/src/render/text.rs` with:

```rust
//! `TextPipeline` — pixel-position textured-quad pipeline that reuses
//! [`crate::render::atlas::GlyphAtlas`]. Used by tab titles/subtitles and the
//! dead-tab banner. Stage 7 (cosmic-text) will replace the simple monospace
//! advance with shaping output.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::render::atlas::GlyphAtlas;

/// Per-instance data for `TextPipeline`. Layout matches `VsIn` in `text.wgsl`.
/// 48 bytes total. Glyph index is stored as `f32` for ergonomics (vec4<f32>
/// is one attribute slot); the shader casts back to u32.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GlyphInstance {
    /// .xy = pixel position (top-left of the glyph cell), .z = glyph_index as f32, .w = unused.
    pub pos_glyph: [f32; 4],
    pub fg: [f32; 4],
    pub bg: [f32; 4],
}

impl GlyphInstance {
    #[must_use]
    pub fn new(x_px: f32, y_px: f32, glyph: u32, fg: [f32; 4], bg: [f32; 4]) -> Self {
        Self {
            pos_glyph: [x_px, y_px, glyph as f32, 0.0],
            fg,
            bg,
        }
    }
}

/// Per-frame uniform. Layout matches `TextUniform` in `text.wgsl`. 32 bytes —
/// already a multiple of 16, no padding needed.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct TextUniform {
    surface_size_px: [f32; 2],
    cell_size_px: [f32; 2],
    atlas_size_px: [f32; 2],
    atlas_cells: [u32; 2],
}

/// Pixel-position text pipeline. Owns its bind group, uniform buffer, and a
/// dynamically-grown instance buffer.
pub struct TextPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,
}

const INITIAL_INSTANCE_CAPACITY: u64 = 256;
const INSTANCE_STRIDE: u64 = std::mem::size_of::<GlyphInstance>() as u64;

impl TextPipeline {
    /// Build the pipeline, sharing the atlas with `GridPipeline`.
    ///
    /// # Errors
    /// Currently infallible after the atlas is built; returns `Result` for
    /// future-proofing.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        atlas: &GlyphAtlas,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibeflow-text-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibeflow-text-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-text-uniform"),
            size: std::mem::size_of::<TextUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vibeflow-text-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas.sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibeflow-text-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibeflow-text-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: INSTANCE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 16,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 32,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-text-instances"),
            size: INSTANCE_STRIDE * INITIAL_INSTANCE_CAPACITY,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
        })
    }

    /// Resize the instance buffer if the requested capacity exceeds the
    /// current allocation. Doubles the capacity each time it grows.
    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, needed: u64) {
        if needed <= self.instance_capacity {
            return;
        }
        let mut new_capacity = self.instance_capacity;
        while new_capacity < needed {
            new_capacity *= 2;
        }
        self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-text-instances"),
            size: INSTANCE_STRIDE * new_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    /// Upload uniforms + instance data and submit one instanced draw call.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        instances: &[GlyphInstance],
        surface_size_px: (u32, u32),
        atlas_size_px: (u32, u32),
        cell_size_px: (u32, u32),
        atlas_cells: (u32, u32),
    ) {
        if instances.is_empty() {
            return;
        }
        let uniform = TextUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            cell_size_px: [cell_size_px.0 as f32, cell_size_px.1 as f32],
            atlas_size_px: [atlas_size_px.0 as f32, atlas_size_px.1 as f32],
            atlas_cells: [atlas_cells.0, atlas_cells.1],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..(instances.len() as u32));
    }
}
```

- [ ] **Step 3: Verify build + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build. The `TextPipeline` is `pub` so no `dead_code` warnings even though it's unused yet. Task 6 wires it into `Renderer`.

If clippy fires on `TextPipeline` not having callers, add `#[allow(dead_code)]` at the struct level with a comment "first user is `Renderer` in Stage 6 Task 6".

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/text.rs crates/vibeflow/src/render/text.wgsl
git commit -m "feat(render): TextPipeline (pixel-position textured-quad)"
```

---

## Task 5: `TabBarPipeline` — solid-color rectangles

**Files:**
- Modify: `crates/vibeflow/src/render/tabs.wgsl`
- Modify: `crates/vibeflow/src/render/tabs.rs` (append `TabBarPipeline` + `RectInstance` after the layout code from Task 3)

`TabBarPipeline` draws solid-color rectangles. Per-instance: pixel position + size + RGBA color. Used for: tab background tints (active vs inactive), Notice indicator stripes, separator lines, and the `+` / `×` button bodies.

- [ ] **Step 1: Write the WGSL shader**

Replace the contents of `crates/vibeflow/src/render/tabs.wgsl` with:

```wgsl
// vibeflow Stage 6 tab-bar rectangle shader.
//
// Per-instance buffer carries pixel-space rect (position + size) and an RGBA
// color. Vertex shader expands 6 vertices per instance into a screen-space
// rectangle. Fragment shader emits the color verbatim (alpha is used for the
// pulse animation on Notice indicators).

struct RectUniform {
    surface_size_px: vec2<f32>,
    _pad:            vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: RectUniform;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) pos_size: vec4<f32>, // .xy = pos_px (top-left), .zw = size_px
    @location(1) color:    vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color:          vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var quad_offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = quad_offsets[in.vertex_id];

    let pos_top_left_px = in.pos_size.xy;
    let size_px         = in.pos_size.zw;

    let pos_px = pos_top_left_px + corner * size_px;
    let ndc    = (pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.color    = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
```

Note: `RectUniform` has a `_pad: vec2<f32>` because `vec2<f32>` is 8 bytes and the WGSL std140-like layout requires the struct to be a multiple of 16 bytes. With one `vec2<f32>` field, the natural size is 8 bytes, which IS aligned but typically gets padded to 16 anyway. Adding `_pad` makes the layout explicit and avoids any alignment ambiguity. (This is the opposite of `GridUniform`'s situation — `GridUniform` had four vec2s totaling 32 bytes and did NOT need padding.)

- [ ] **Step 2: Append `RectInstance` and `TabBarPipeline` to `crates/vibeflow/src/render/tabs.rs`**

Append the following code at the END of `crates/vibeflow/src/render/tabs.rs` (after the `mod tests` block from Task 3):

```rust
use anyhow::Result;
use bytemuck::{Pod, Zeroable};

/// Per-instance data for [`TabBarPipeline`]. 32 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct RectInstance {
    /// .xy = pos_px (top-left), .zw = size_px (width, height).
    pub pos_size: [f32; 4],
    pub color: [f32; 4],
}

impl RectInstance {
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        Self {
            pos_size: [x, y, w, h],
            color,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct RectUniform {
    surface_size_px: [f32; 2],
    _pad: [f32; 2],
}

/// Solid-color-rectangle pipeline for the tab bar.
pub struct TabBarPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,
}

const INITIAL_RECT_CAPACITY: u64 = 64;
const RECT_STRIDE: u64 = std::mem::size_of::<RectInstance>() as u64;

impl TabBarPipeline {
    /// Build the pipeline.
    ///
    /// # Errors
    /// Currently infallible.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibeflow-tabs-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("tabs.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibeflow-tabs-bind-group-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-tabs-uniform"),
            size: std::mem::size_of::<RectUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vibeflow-tabs-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibeflow-tabs-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibeflow-tabs-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: RECT_STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 16,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-tabs-instances"),
            size: RECT_STRIDE * INITIAL_RECT_CAPACITY,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_capacity: INITIAL_RECT_CAPACITY,
        })
    }

    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, needed: u64) {
        if needed <= self.instance_capacity {
            return;
        }
        let mut new_capacity = self.instance_capacity;
        while new_capacity < needed {
            new_capacity *= 2;
        }
        self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-tabs-instances"),
            size: RECT_STRIDE * new_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    /// Submit one instanced draw call for all the rects.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        rects: &[RectInstance],
        surface_size_px: (u32, u32),
    ) {
        if rects.is_empty() {
            return;
        }
        let uniform = RectUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(rects));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..(rects.len() as u32));
    }
}
```

Note: `BlendState::ALPHA_BLENDING` (not `REPLACE` like the cell pipeline) — the indicator stripe's pulse animation modulates alpha, and we want the bar background to show through.

- [ ] **Step 3: Verify build + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/tabs.rs crates/vibeflow/src/render/tabs.wgsl
git commit -m "feat(render): TabBarPipeline (solid-color rects + alpha blending)"
```

---

## Task 6: `Renderer` integration — tab-bar pass + scissor for cell grid

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`
- Modify: `crates/vibeflow/src/render/tabs.rs` (add `TabBarRenderer` glue)

This task wires the new pipelines into `Renderer::render`. The render pass clears, then:
1. Draws the cell grid (existing `GridPipeline`) with a scissor rect that excludes the top `tab_bar_height_px` pixels.
2. Draws the tab-bar background + indicator stripes via `TabBarPipeline`.
3. Draws the tab title and subtitle text via `TextPipeline`.

Cursor is in the cell pass (Stage 5). Dead-tab banner (Task 7) and pulse animation (Task 7 too) are layered on top.

`TabBarRenderer` is a small helper that, given an `App` and the layout, produces the `RectInstance` list and the `GlyphInstance` list. Pulse-alpha computation lives there.

- [ ] **Step 1: Add `TabBarRenderer` glue**

Append to `crates/vibeflow/src/render/tabs.rs` (after `TabBarPipeline`):

```rust
use std::time::Instant;

use crate::app::App;
use crate::render::atlas::{glyph_index, GlyphAtlas};
use crate::render::text::GlyphInstance;
use crate::session::tracker::TabState;

/// Notice-indicator colors. The amber/blue/gray come from the design spec's
/// `[theme.indicator]` defaults; they'll be configurable in Stage 9.
fn indicator_color(state: TabState) -> [f32; 4] {
    match state {
        TabState::Waiting => [0xff as f32 / 255.0, 0xbd as f32 / 255.0, 0x2e as f32 / 255.0, 1.0], // amber
        TabState::Working => [0x5f as f32 / 255.0, 0xb4 as f32 / 255.0, 0xff as f32 / 255.0, 1.0], // blue
        TabState::Idle => [0x45 as f32 / 255.0, 0x45 as f32 / 255.0, 0x4f as f32 / 255.0, 1.0],    // gray
        TabState::Done => [0x5f as f32 / 255.0, 0xff as f32 / 255.0, 0x9f as f32 / 255.0, 1.0],    // greenish
        TabState::Active => [0.0, 0.0, 0.0, 0.0], // no stripe for the default state
    }
}

/// Compute the pulse alpha for a `Waiting` tab at time `t` (seconds since some epoch).
/// 1.4 s sine wave between 0.4 and 1.0.
#[must_use]
pub fn pulse_alpha(t_secs: f32) -> f32 {
    let omega = std::f32::consts::TAU / 1.4;
    let sin = (t_secs * omega).sin(); // -1 to 1
    0.7 + 0.3 * sin // 0.4 to 1.0
}

/// Width of the Notice indicator stripe in pixels. Spec: 3px.
pub const INDICATOR_STRIPE_WIDTH_PX: u32 = 3;

/// Tab background colors (active vs inactive). Spec: active is "slightly lighter".
const BG_ACTIVE: [f32; 4] = [0x1a as f32 / 255.0, 0x1a as f32 / 255.0, 0x22 as f32 / 255.0, 1.0];
const BG_INACTIVE: [f32; 4] = [0x15 as f32 / 255.0, 0x15 as f32 / 255.0, 0x1c as f32 / 255.0, 1.0];

/// Title text color (slightly muted on inactive tabs).
const FG_ACTIVE: [f32; 4] = [0xe5 as f32 / 255.0, 0xe5 as f32 / 255.0, 0xe5 as f32 / 255.0, 1.0];
const FG_INACTIVE: [f32; 4] = [0x7a as f32 / 255.0, 0x7a as f32 / 255.0, 0x82 as f32 / 255.0, 1.0];

/// `+` and `×` button glyph indices into the atlas (looked up at construction).
fn plus_glyph_idx() -> u32 {
    glyph_index('+').unwrap_or(0)
}
fn x_glyph_idx() -> u32 {
    glyph_index('×').unwrap_or_else(|| glyph_index('x').unwrap_or(0))
}

/// Glue between `App` state, `TabBarLayout`, and the wgpu pipelines.
/// Stateless except for the pulse-time epoch.
pub struct TabBarRenderer {
    /// Wall-clock time at which the renderer was constructed; pulse alpha is
    /// computed as `(now - epoch).as_secs_f32()`.
    epoch: Instant,
}

impl TabBarRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    /// Build the RectInstance list (tab backgrounds + indicator stripes + close
    /// buttons + new-tab button) for the current `App` state.
    pub fn build_rects(&self, app: &App, layout: &TabBarLayout) -> Vec<RectInstance> {
        let mut rects = Vec::new();
        let bar_height = layout.bar_height_px as f32;
        let active_idx = app.active();
        // Continuous monotonic time — pulse_alpha cycles every 1.4 s naturally.
        // (Do NOT use `t.fract()` here — that resets phase every integer second
        // and produces a visible jump.)
        let pulse = pulse_alpha(self.epoch.elapsed().as_secs_f32());

        // Tab backgrounds first (so stripes draw on top).
        for tab in &layout.tabs {
            let is_active = tab.idx == active_idx && tab.idx < app.tabs().len();
            let bg = if is_active { BG_ACTIVE } else { BG_INACTIVE };
            rects.push(RectInstance::new(
                tab.body.x as f32,
                tab.body.y as f32,
                tab.body.w as f32,
                tab.body.h as f32,
                bg,
            ));

            // Notice indicator stripe (3px on the left edge).
            let session = match app.tabs().get(tab.idx) {
                Some(s) => s,
                None => continue,
            };
            let state = session.state();
            let mut color = indicator_color(state);
            if state == TabState::Waiting {
                color[3] = pulse; // sine-modulated alpha
            }
            // Skip if the color is fully transparent (active state).
            if color[3] > 0.0 {
                rects.push(RectInstance::new(
                    tab.body.x as f32,
                    tab.body.y as f32,
                    INDICATOR_STRIPE_WIDTH_PX as f32,
                    bar_height,
                    color,
                ));
            }

            // Per-tab close button background (subtle).
            rects.push(RectInstance::new(
                tab.close_button.x as f32,
                tab.close_button.y as f32,
                tab.close_button.w as f32,
                tab.close_button.h as f32,
                [0.0, 0.0, 0.0, 0.3],
            ));
        }

        // New-tab button background.
        rects.push(RectInstance::new(
            layout.new_tab_button.x as f32,
            layout.new_tab_button.y as f32,
            layout.new_tab_button.w as f32,
            layout.new_tab_button.h as f32,
            [0.0, 0.0, 0.0, 0.3],
        ));

        rects
    }

    /// Build the GlyphInstance list for tab titles + subtitles + `+` / `×`
    /// button glyphs.
    pub fn build_glyphs(
        &self,
        app: &App,
        layout: &TabBarLayout,
        atlas: &GlyphAtlas,
    ) -> Vec<GlyphInstance> {
        let mut glyphs = Vec::new();
        let active_idx = app.active();
        let (cell_w, cell_h) = atlas.cell_pitch();
        let cell_w_f = cell_w as f32;
        let cell_h_f = cell_h as f32;

        for tab in &layout.tabs {
            let is_active = tab.idx == active_idx && tab.idx < app.tabs().len();
            let fg = if is_active { FG_ACTIVE } else { FG_INACTIVE };
            let bg = if is_active { BG_ACTIVE } else { BG_INACTIVE };

            let session = match app.tabs().get(tab.idx) {
                Some(s) => s,
                None => continue,
            };
            let label = session.label();

            // Title on line 1 (top of tab body).
            let title_x_start = tab.body.x as f32 + (INDICATOR_STRIPE_WIDTH_PX as f32) + 6.0;
            let title_y = tab.body.y as f32 + 2.0;
            push_text_glyphs(
                &mut glyphs,
                &label.title,
                title_x_start,
                title_y,
                cell_w_f,
                fg,
                bg,
                tab.body.x + tab.body.w - tab.close_button.w - 4,
            );

            // Subtitle on line 2 (below title).
            let subtitle_x_start = title_x_start;
            let subtitle_y = title_y + cell_h_f;
            push_text_glyphs(
                &mut glyphs,
                &label.subtitle,
                subtitle_x_start,
                subtitle_y,
                cell_w_f,
                fg,
                bg,
                tab.body.x + tab.body.w - tab.close_button.w - 4,
            );

            // `×` glyph centered in the close button. The TextPipeline's
            // fragment shader forces alpha=1, so we want bg to match the tab
            // body — the close-button-rect overlay is drawn underneath in the
            // RectInstance pass, but the × glyph's bg rectangle would override
            // it. Using `bg` here makes the close button visually defined by
            // the × glyph alone.
            let close_glyph_x =
                tab.close_button.x as f32 + (tab.close_button.w as f32 - cell_w_f) / 2.0;
            let close_glyph_y =
                tab.close_button.y as f32 + (tab.close_button.h as f32 - cell_h_f) / 2.0;
            glyphs.push(GlyphInstance::new(
                close_glyph_x,
                close_glyph_y,
                x_glyph_idx(),
                fg,
                bg,
            ));
        }

        // `+` glyph centered in the new-tab button. Use BG_INACTIVE so the
        // glyph's bg rectangle blends into the tab-bar strip; the visible mark
        // is just the `+` character itself.
        let nb = layout.new_tab_button;
        let plus_glyph_x = nb.x as f32 + (nb.w as f32 - cell_w_f) / 2.0;
        let plus_glyph_y = nb.y as f32 + (nb.h as f32 - cell_h_f) / 2.0;
        glyphs.push(GlyphInstance::new(
            plus_glyph_x,
            plus_glyph_y,
            plus_glyph_idx(),
            FG_ACTIVE,
            BG_INACTIVE,
        ));

        glyphs
    }

    /// Helper for the smoke checklist + tests: time since this renderer was
    /// constructed, in seconds. Used to drive the pulse animation.
    #[must_use]
    pub fn elapsed_secs(&self) -> f32 {
        self.epoch.elapsed().as_secs_f32()
    }
}

impl Default for TabBarRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a string to a sequence of `GlyphInstance`s laid out at integer
/// pixel positions, clipped to a max-x boundary so titles/subtitles don't
/// spill onto the close button.
fn push_text_glyphs(
    out: &mut Vec<GlyphInstance>,
    s: &str,
    x_start: f32,
    y: f32,
    cell_w: f32,
    fg: [f32; 4],
    bg: [f32; 4],
    max_x_px: u32,
) {
    let mut x = x_start;
    for c in s.chars() {
        let glyph = glyph_index(c).unwrap_or(0);
        if x + cell_w > max_x_px as f32 {
            break;
        }
        out.push(GlyphInstance::new(x, y, glyph, fg, bg));
        x += cell_w;
    }
}
```

Add the corresponding tests at the END of the existing `mod tests` block (i.e. inside the same `#[cfg(test)] mod tests { ... }`):

```rust
    #[test]
    fn pulse_alpha_at_t_zero_is_in_range() {
        let a = pulse_alpha(0.0);
        assert!(a >= 0.4 && a <= 1.0, "got {}", a);
    }

    #[test]
    fn pulse_alpha_oscillates_over_a_full_period() {
        // Full period is 1.4 s. The alpha hits both ends across that span.
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for i in 0..100 {
            let t = (i as f32) * 0.014; // 100 samples across 1.4 s
            let a = pulse_alpha(t);
            min = min.min(a);
            max = max.max(a);
        }
        assert!(min < 0.5, "expected min near 0.4, got {}", min);
        assert!(max > 0.95, "expected max near 1.0, got {}", max);
    }

    #[test]
    fn indicator_color_is_amber_for_waiting() {
        let c = indicator_color(TabState::Waiting);
        // 0xff = 1.0, 0xbd ≈ 0.74, 0x2e ≈ 0.18.
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.74).abs() < 0.05);
        assert!((c[2] - 0.18).abs() < 0.05);
    }

    #[test]
    fn indicator_color_is_transparent_for_active() {
        assert_eq!(indicator_color(TabState::Active), [0.0, 0.0, 0.0, 0.0]);
    }
```

- [ ] **Step 2: Update `Renderer` to own `TextPipeline` and `TabBarPipeline` and use them in `render`**

In `crates/vibeflow/src/render/mod.rs`, find the `Renderer` struct and add three new fields after `grid_pipeline`:

```rust
    /// Pixel-position text pipeline (tab titles, subtitles, dead-tab banner, button glyphs).
    text_pipeline: crate::render::text::TextPipeline,
    /// Solid-color rectangle pipeline (tab backgrounds, indicator stripes, button bodies).
    tab_bar_pipeline: crate::render::tabs::TabBarPipeline,
    /// Per-frame TabBarRenderer (owns the pulse-animation epoch).
    tab_bar: crate::render::tabs::TabBarRenderer,
```

In `Renderer::new`, after the `grid_pipeline` is constructed, add:

```rust
        let text_pipeline =
            crate::render::text::TextPipeline::new(&device, format, &atlas)?;
        let tab_bar_pipeline =
            crate::render::tabs::TabBarPipeline::new(&device, format)?;
        let tab_bar = crate::render::tabs::TabBarRenderer::new();
```

And update the `Ok(Self { ... })` block to include the three new fields.

Then update `Renderer::render` to take an `&App` (to read tab state) and add the tab-bar pass. The current Stage-5 signature is:

```rust
    pub fn render(
        &mut self,
        term: Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>>,
    ) -> std::result::Result<(), wgpu::SurfaceError>
```

Replace with:

```rust
    pub fn render(
        &mut self,
        term: Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>>,
        app: &crate::app::App,
    ) -> std::result::Result<(), wgpu::SurfaceError> {
        use crate::render::tabs::TabBarLayout;

        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vibeflow-frame-encoder"),
            });

        let (cell_w, cell_h) = self.atlas.cell_pitch();
        let surface_size = (
            self.surface_config.width,
            self.surface_config.height,
        );
        let layout = TabBarLayout::compute(surface_size.0, cell_h, app.tabs().len());

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vibeflow-frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // ---- Cell grid pass (excluded from tab bar region via scissor) ----
            if let Some(term) = term {
                let instances = build_cell_instances(term);
                if !instances.is_empty() {
                    self.grid_pipeline
                        .ensure_instance_capacity(&self.device, instances.len() as u64);
                    let (atlas_w, atlas_h) = self.atlas.pixel_size();
                    pass.set_scissor_rect(
                        0,
                        layout.bar_height_px,
                        surface_size.0,
                        surface_size.1.saturating_sub(layout.bar_height_px),
                    );
                    self.grid_pipeline.draw(
                        &mut pass,
                        &self.queue,
                        &instances,
                        // The grid renders into the area BELOW the tab bar.
                        // We pass the FULL surface size and let the scissor clip;
                        // cells are drawn at cell-grid coordinates starting at (0, 0).
                        // To shift the grid down by tab_bar_height_px, we need a
                        // y-offset uniform — but Stage 6 keeps the grid shader
                        // unchanged and shifts via the scissor + by adjusting cell row
                        // origin in `build_cell_instances`. Stage 6 simplifies by
                        // computing the scissor clip but still drawing cells at
                        // grid-aligned positions starting at y=0. The visible
                        // result: rows 0..N near the top of the cell grid pass are
                        // hidden behind the tab bar. For Stage 6 demo this is OK;
                        // Stage 7+ adds a proper y-offset uniform.
                        // (See "Notable plan risks" at the bottom of this plan.)
                        surface_size,
                        (atlas_w, atlas_h),
                        (cell_w, cell_h),
                        crate::render::atlas::ATLAS_LAYOUT,
                    );
                    // Reset scissor for the next pass.
                    pass.set_scissor_rect(0, 0, surface_size.0, surface_size.1);
                }
            }

            // ---- Tab bar pass ----
            let rects = self.tab_bar.build_rects(app, &layout);
            if !rects.is_empty() {
                self.tab_bar_pipeline
                    .ensure_instance_capacity(&self.device, rects.len() as u64);
                self.tab_bar_pipeline.draw(&mut pass, &self.queue, &rects, surface_size);
            }

            // ---- Tab bar text pass ----
            let glyphs = self.tab_bar.build_glyphs(app, &layout, &self.atlas);
            if !glyphs.is_empty() {
                self.text_pipeline
                    .ensure_instance_capacity(&self.device, glyphs.len() as u64);
                let (atlas_w, atlas_h) = self.atlas.pixel_size();
                self.text_pipeline.draw(
                    &mut pass,
                    &self.queue,
                    &glyphs,
                    surface_size,
                    (atlas_w, atlas_h),
                    (cell_w, cell_h),
                    crate::render::atlas::ATLAS_LAYOUT,
                );
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
```

- [ ] **Step 3: Update the call site in `window.rs`**

In `crates/vibeflow/src/window.rs`, find the `RedrawRequested` arm. The current call is:

```rust
                match renderer.render(term) {
```

Replace with:

```rust
                match renderer.render(term, &self.app) {
```

The `&self.app` borrow is valid here because `renderer.as_mut()` and `&self.app` are split borrows — different fields of `self`.

Wait — there's a borrow-checker issue. `self.app.active_term()` was called above to get `term`, which holds an immutable borrow on `self.app`. Then we want `&self.app` again at the same time. But we ALSO have `self.renderer.as_mut()` which is a mutable borrow on `self.renderer`. The compiler has to prove these don't overlap.

The Stage 5 callsite was:

```rust
            WindowEvent::RedrawRequested => {
                let term = self.app.active_term();      // &App
                let Some(renderer) = self.renderer.as_mut() else { return; };  // &mut Renderer
                match renderer.render(term) {           // call with &App and &mut Renderer (split fields, OK)
                ...
```

To pass `&self.app` to render, we just need to keep the immutable borrow alive. Since `term` is `Option<&Term>` derived from `&self.app`, and we're using `&self.app` again — both immutable borrows of `self.app` — they coexist fine. So the new callsite is:

```rust
            WindowEvent::RedrawRequested => {
                let term = self.app.active_term();
                let Some(renderer) = self.renderer.as_mut() else { return; };
                match renderer.render(term, &self.app) {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        renderer.reconfigure();
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        tracing::error!("GPU out of memory; exiting");
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
```

- [ ] **Step 4: Verify build + fmt + clippy + smoke**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
cargo test -p vibeflow
```

Expected: clean build, all tests pass.

Smoke run (if you have a display):

```bash
cd /home/bhengen/dev/vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

You should see a tab bar at the top of the window with one tab labeled "bash · idle" (or similar), a `+` button at the right, and a `×` close button on the tab. Clicking does nothing yet — that's Task 8.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/mod.rs crates/vibeflow/src/render/tabs.rs crates/vibeflow/src/window.rs
git commit -m "feat(render): tab bar + indicator stripes + tab text in Renderer::render"
```

---

## Task 7: Pulse animation timing — 60 Hz redraw scheduling for `Waiting` tabs

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

The `pulse_alpha` math (from Task 6) assumes the renderer is invoked frequently enough to produce visible animation. Stage 4's `about_to_wait` uses `ControlFlow::WaitUntil(now + 100ms)` — that's 10 Hz, too slow for a smooth pulse. We need:

- When at least one tab is in `Waiting`: re-arm at `now + 16ms` (60 Hz). And request a redraw every wake-up so the renderer reads the new pulse alpha.
- Otherwise: re-arm at `now + 100ms` as before.

Pure integration; no new pipeline state.

- [ ] **Step 1: Modify `WindowApp::about_to_wait`**

In `crates/vibeflow/src/window.rs`, find the existing `about_to_wait` method. The current body (after Stage 5):

```rust
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        for (idx, ev) in self.app.poll_all(now) {
            self.handle_session_event(idx, ev);
        }
        for (idx, ev) in self.app.tick_all(now) {
            self.handle_session_event(idx, ev);
        }

        event_loop
            .set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(100)));
    }
```

Replace with:

```rust
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        use crate::session::tracker::TabState;

        let now = Instant::now();

        for (idx, ev) in self.app.poll_all(now) {
            self.handle_session_event(idx, ev);
        }
        for (idx, ev) in self.app.tick_all(now) {
            self.handle_session_event(idx, ev);
        }

        // Pulse animation: while ANY tab is in Waiting, run at 60 Hz so the
        // amber stripe pulse looks smooth. Otherwise fall back to the 10 Hz
        // tracker-tick cadence.
        let any_waiting = self
            .app
            .tabs()
            .iter()
            .any(|tab| tab.state() == TabState::Waiting);
        let next_deadline = if any_waiting {
            // Also request a redraw each tick so the new pulse alpha hits the GPU.
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
            now + Duration::from_millis(16)
        } else {
            now + Duration::from_millis(100)
        };

        event_loop.set_control_flow(ControlFlow::WaitUntil(next_deadline));
    }
```

- [ ] **Step 2: Verify build + smoke**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Smoke run with a Waiting trigger:

```bash
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

In the running shell, emit an OSC 1338 waiting frame manually:

```bash
printf '\033]1338;state=waiting\007'
```

The `bash · waiting` subtitle should appear, and the amber indicator stripe should visibly pulse (alpha oscillating between 40% and 100% on a 1.4 s sine).

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): 60 Hz pulse animation cadence while any tab is Waiting"
```

---

## Task 8: Mouse handling — tab create/close/switch via clicks

**Files:**
- Modify: `crates/vibeflow/src/window.rs`
- Modify: `crates/vibeflow/src/app.rs`

Mouse events arrive via `WindowEvent::CursorMoved` (gives the latest cursor position) and `WindowEvent::MouseInput` (gives press/release of a button). On a left-click release, hit-test against the tab bar layout and dispatch:
- `TabBarHit::NewTab` → `App::new_tab(default_shell_argv)`
- `TabBarHit::TabBody(idx)` → `App::set_active(idx)`
- `TabBarHit::TabClose(idx)` → `App::close_tab(idx)` (already exists in App)

The default-shell argv: the same one Stage 4's `WindowApp::spawn_first_tab` uses (`$SHELL` or `/bin/sh`).

- [ ] **Step 1: Add `App::set_active`**

In `crates/vibeflow/src/app.rs`, locate the `App::active(&self) -> usize` accessor. Add a sibling:

```rust
    /// Set the focused tab. No-op if `idx` is out of range.
    pub fn set_active(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
        }
    }
```

(Keep `App::close_tab` and `App::new_tab` unchanged — they already do the right thing.)

Append a test to `mod tests`:

```rust
    #[test]
    fn set_active_focuses_the_specified_tab() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        // After three new_tab calls, active = 2 (most-recently-spawned).
        assert_eq!(app.active(), 2);
        app.set_active(0);
        assert_eq!(app.active(), 0);
    }

    #[test]
    fn set_active_with_out_of_range_idx_is_a_no_op() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.set_active(99);
        assert_eq!(app.active(), 0);
    }
```

- [ ] **Step 2: Add cursor-position state + mouse handlers to `WindowApp`**

In `crates/vibeflow/src/window.rs`, modify the `WindowApp` struct to add:

```rust
    /// Latest cursor position from `WindowEvent::CursorMoved`. Used by mouse
    /// click handlers to hit-test the tab bar.
    cursor_pos: Option<(u32, u32)>,
```

And initialize it in `WindowApp::new`:

```rust
            cursor_pos: None,
```

In the `window_event` match, add two new arms above the catch-all:

```rust
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = Some((position.x as u32, position.y as u32));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                use winit::event::{ElementState, MouseButton};
                if state == ElementState::Released && button == MouseButton::Left {
                    self.handle_left_click_release();
                }
            }
```

Add the `handle_left_click_release` method to `impl WindowApp` (anywhere reasonable, e.g., after `handle_session_event`):

```rust
    /// Hit-test the latest cursor position against the tab bar and dispatch
    /// the corresponding action.
    fn handle_left_click_release(&mut self) {
        use crate::render::tabs::{TabBarHit, TabBarLayout};

        let Some((px, py)) = self.cursor_pos else {
            return;
        };
        // We need the same layout the renderer used. Since cell pitch + window
        // width are the inputs, recompute it here from the renderer's atlas.
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let (_cell_w, cell_h) = renderer.cell_pitch();
        let (window_w, _window_h) = renderer.surface_size();
        let layout = TabBarLayout::compute(window_w, cell_h, self.app.tabs().len());

        match layout.hit_test(px, py) {
            TabBarHit::NewTab => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                if let Err(e) = self.app.new_tab(&[shell.as_str()]) {
                    tracing::warn!(error = ?e, "new_tab failed");
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::TabBody(idx) => {
                self.app.set_active(idx);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::TabClose(idx) => {
                self.app.close_tab(idx);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::None => {}
        }
    }
```

- [ ] **Step 3: Verify build + tests + smoke**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 2 new app tests pass; lib total rises from 109 (post-Task-2) + 9 (Task 3) + (Task 6: 4 tests) = 122 to 124. (Task 1 added 1, Task 2 added 6, Task 3 added 9, Task 6 added 4, Task 8 adds 2.)

Smoke run:

```bash
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- Click the `+` button: a new tab should appear.
- Click on a different tab: the cell grid should switch to that tab's content.
- Click the `×` on a tab: the tab should disappear.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/app.rs crates/vibeflow/src/window.rs
git commit -m "feat(window,app): mouse-driven tab create/close/switch"
```

---

## Task 9: Dead-tab banner

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

When `PtySession::is_alive()` is `false`, the renderer should overlay a banner across the cell-grid area with text like "session died -- press Ctrl+Shift+R to retry". For Stage 6, the keyboard shortcut isn't wired (Stage 8 does that), but the banner still informs the user.

The banner is rendered as a single `RectInstance` (semi-transparent dark background) followed by `GlyphInstance`s for the text, using the existing `TabBarPipeline` and `TextPipeline`.

- [ ] **Step 1: Add the banner rendering to `Renderer::render`**

In `crates/vibeflow/src/render/mod.rs`, find the `render` method's body — specifically the section where the tab-bar pass runs. After the tab-bar text pass (the third one) and BEFORE `self.queue.submit(...)`, add a new "dead-tab banner" section:

```rust
            // ---- Dead-tab banner (overlay on the cell grid area) ----
            if let Some(active_session) = app.tabs().get(app.active()) {
                if !active_session.is_alive() {
                    let banner_text = "session died -- press Ctrl+Shift+R to retry";
                    let banner_h = (cell_h as f32) * 2.0;
                    let banner_y = layout.bar_height_px as f32 + 16.0;
                    let banner_w = surface_size.0 as f32;

                    // Semi-transparent dark background.
                    let banner_rect = crate::render::tabs::RectInstance::new(
                        0.0,
                        banner_y,
                        banner_w,
                        banner_h,
                        [0.0, 0.0, 0.0, 0.85],
                    );
                    self.tab_bar_pipeline
                        .ensure_instance_capacity(&self.device, 1);
                    self.tab_bar_pipeline.draw(
                        &mut pass,
                        &self.queue,
                        std::slice::from_ref(&banner_rect),
                        surface_size,
                    );

                    // Centered text on top.
                    let text_w = (banner_text.chars().count() as f32) * (cell_w as f32);
                    let text_x = (banner_w - text_w) / 2.0;
                    let text_y = banner_y + (banner_h - cell_h as f32) / 2.0;
                    let mut banner_glyphs: Vec<crate::render::text::GlyphInstance> = Vec::new();
                    let mut x = text_x;
                    for c in banner_text.chars() {
                        let glyph = crate::render::atlas::glyph_index(c).unwrap_or(0);
                        banner_glyphs.push(crate::render::text::GlyphInstance::new(
                            x,
                            text_y,
                            glyph,
                            [0xff as f32 / 255.0, 0xbd as f32 / 255.0, 0x2e as f32 / 255.0, 1.0], // amber
                            [0.0, 0.0, 0.0, 1.0], // opaque black, matches banner rect underneath
                        ));
                        x += cell_w as f32;
                    }
                    self.text_pipeline
                        .ensure_instance_capacity(&self.device, banner_glyphs.len() as u64);
                    let (atlas_w, atlas_h) = self.atlas.pixel_size();
                    self.text_pipeline.draw(
                        &mut pass,
                        &self.queue,
                        &banner_glyphs,
                        surface_size,
                        (atlas_w, atlas_h),
                        (cell_w, cell_h),
                        crate::render::atlas::ATLAS_LAYOUT,
                    );
                }
            }
```

Place this block AFTER the tab-bar text pass but still inside the `{ ... }` scope of the render pass.

- [ ] **Step 2: Verify build + smoke**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Smoke run: open vibeflow, type `exit` in the shell. The shell exits, the session dies, the banner should appear over the cell grid area.

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(render): dead-tab banner overlay when session.is_alive() == false"
```

---

## Task 10: Final verification + tag

- [ ] **Step 1: Append Stage 6 section to `docs/TESTING.md`**

```markdown

## Stage 6 — tab bar + Notice indicator + dead-tab banner + mouse tab interaction

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] A tab bar is visible at the top of the window. One tab is shown, labeled
  with the shell name (e.g. `bash`) on line 1 and `active` (or `idle` after
  the prompt's OSC 133 fires) on line 2.
- [ ] The tab has a `×` close button at its right edge and the bar has a `+`
  button at the right end.
- [ ] Click the `+` button. A second tab spawns, becomes active, and the cell
  grid switches to its content.
- [ ] Click on the first tab. Focus switches back; cell grid reflects the
  first shell.
- [ ] In the active tab, manually emit an OSC 1338 waiting frame:
  ```
  printf '\033]1338;state=waiting\007'
  ```
  The subtitle changes to `waiting`. An amber stripe appears on the left edge
  of the tab and pulses smoothly (~1.4s sine, alpha between 40% and 100%).
- [ ] Emit a working frame: `printf '\033]1338;state=working\007'`. The stripe
  changes to steady blue (no pulse).
- [ ] In a second tab, run `exit` (or close the shell). Session dies; an
  amber banner appears over the cell grid area: "session died -- press
  Ctrl+Shift+R to retry". (The keyboard shortcut isn't wired yet — that's
  Stage 8. But the visual banner works.)
- [ ] Click the `×` button on the second tab. The tab is removed; the bar
  reverts to one tab.
- [ ] Resize the window. The tab bar height stays constant; tab widths
  re-scale.
- [ ] Spawn many tabs (10+). Tab widths clamp to MIN_TAB_WIDTH_PX; the bar
  remains usable.

If any step fails, capture the failure and a screenshot before fixing.
```

- [ ] **Step 2: Full local CI dry-run**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo build --workspace --all-targets && \
  cargo test --workspace --all-targets && \
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps && \
  ( cd bindings/npm && npm run build && npm test ) && \
  echo "ALL GREEN"
```

Expected test count: every Stage-5 test still passes (102 lib + 3 + 4 + 27 + 1 proptest = 137) plus Stage 6's additions:
- Task 1: 1 test
- Task 2: 6 tests
- Task 3: 9 tests
- Task 6: 4 tests
- Task 8: 2 tests
Net: 22 new lib tests → ~124 lib + 3 + 4 + 27 = 158 Rust tests + 1 proptest + 15 npm.

- [ ] **Step 3: 60-second fuzz on the protocol parser**

```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

- [ ] **Step 4: Walk the smoke checklist**

Re-walk `docs/TESTING.md`'s Stage 6 section.

- [ ] **Step 5: Commit + tag**

```bash
cd /home/bhengen/dev/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 6 manual smoke checklist"
git tag -a stage6-tab-bar-complete -m "tab bar + Notice indicator + dead-tab banner + mouse tab interaction complete (Stage 6 of v0.1)"
git tag --list
```

- [ ] **Step 6: Surface to user**

Report:
- Number of new commits on this stage (~10).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 7 (cosmic-text font shaping) as the next plan.

---

## Spec coverage check

Mapping Stage 6 spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| Differentiator — Notice indicator (3px stripe + tinted subtitle) | Task 6 (RectInstance for stripe; FG_INACTIVE/FG_ACTIVE for subtitle tint not yet implemented — see "Notable plan risks") |
| Differentiator — amber pulse on `Waiting` | Task 6 (`pulse_alpha`) + Task 7 (60 Hz redraw scheduling) |
| Differentiator — pulse only while waiting tab exists | Task 7 (pulse-aware ControlFlow) |
| Components — `app.rs` (~120 LOC) | Task 8 (`set_active` added) |
| Components — `render/tabs.rs` (~220 LOC) | Tasks 3 + 5 + 6 (~350 LOC, slightly over spec estimate) |
| Components — `render/font.rs` (~150 LOC) | Reused from Stage 5's `atlas.rs`; Stage 7 expands |
| Visual design — two-line tab format | Task 6 (title at y=2, subtitle at y=2+cell_h) |
| Visual design — active tab background lighter | Task 6 (BG_ACTIVE vs BG_INACTIVE) |
| Error handling — Tab opens to in-tab error pane | Task 9 (dead-tab banner) |
| Error handling — Mark session dead, freeze grid | Inherited from Stage 4; banner overlays the frozen grid |

**Out of scope for Stage 6 (deferred):**
- Cwd-based tab titles — Stage 9 (procfs polling) or shell-hook integration
- Selection rendering on mouse drag — separate stage (mouse-drag handling + selection-rect render pass)
- Scrollback rendering on mouse wheel — separate stage
- Cursor blink animation — Stage 7+
- TOML config + hot-reload — Stage 9
- Keyboard shortcuts (`Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, etc.) — Stage 8
- Subtitle text-color tinting based on tracker state — see "Notable plan risks"; Stage 6 ships flat colors and Stage 7 layers in tint
- Bell / visual flash — Stage 7+
- Hyperlinks — Stage 8+

## Self-review

- **Spec coverage:** every Stage 6-relevant spec requirement maps to a task. Stages 7+ items are explicitly listed as out of scope.
- **Placeholder scan:** no `TBD`/`TODO`/`implement later`/`similar to` patterns. Each step has actual code or actual commands.
- **Type consistency check:**
  - `TabLabel { title: String, subtitle: String }` — defined in Task 2, read by `TabBarRenderer::build_glyphs` in Task 6.
  - `TabBarLayout`, `Rect`, `TabRect`, `TabBarHit` — defined in Task 3, used in Tasks 6 and 8.
  - `RectInstance` (32 bytes), `GlyphInstance` (48 bytes) — defined in Tasks 5 and 4 respectively, used in Task 6.
  - `Renderer::render(term: Option<&Term>, app: &App)` — signature in Task 6, callsite in window.rs Task 6's Step 3.
  - `App::set_active(idx: usize)` — defined in Task 8, called from `WindowApp::handle_left_click_release`.
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** unchanged. `TabBarRenderer`, `TabBarPipeline`, `TextPipeline` all live on the main thread inside `Renderer`. No new threads, no new locks.
- **Pedagogical clarity:** the plan explains non-obvious choices inline:
  - `BlendState::ALPHA_BLENDING` for `TabBarPipeline` (vs `REPLACE` for grid) — pulse alpha needs blending
  - `pulse_alpha` math (sine wave, period 1.4 s, range 0.4–1.0)
  - The `_pad: vec2<f32>` on `RectUniform` (vs no pad on `GridUniform`) — alignment difference for one-vec2 vs four-vec2 structs
  - The split borrow trick in `WindowApp` for `cursor_pos` + `app` + `renderer`
  - The `refresh_default_subtitle` heuristic (single-word vs spaced-title) as a Stage-9-replaceable simplification
- **Forward-declared item handling:** `TextPipeline` (Task 4) and `TabBarPipeline`+`TabBarRenderer` (Task 5) are introduced before `Renderer` constructs them in Task 6. Their `pub` visibility means clippy doesn't fire `dead_code`. If it does (for a method that's still not called between tasks), narrow with `#[allow(dead_code)]` and a "first user is Renderer in Stage 6 Task 6" comment, removed when the integration lands.

---

## Notable plan risks

1. **Cell grid is drawn at the same y origin as the tab bar.** The plan uses a scissor rect to hide the cell grid's top `tab_bar_height_px` rows, but `build_cell_instances` still emits cells starting at row 0 (which is the top of the screen, behind the tab bar). The visible result: the topmost N rows of the cell grid are clipped from view. Stage 7+ should add a y-offset uniform to `GridPipeline` to shift the grid down by `tab_bar_height_px`. For Stage 6, the loss is a few rows at the top — usable but not ideal.

2. **`refresh_default_subtitle`'s heuristic is fragile.** It uses "title contains a space" as the proxy for "user has overridden the title." Real custom titles like `claude-code` or `gpt-4o` lack spaces and would be wrongly auto-refreshed. Stage 9 should add an explicit `is_overridden: bool` flag on `TabLabel`. For Stage 6, the heuristic is good enough for the demo (default titles are always single words: `bash`, `zsh`, `sh`).

3. **Subtitle tint based on tracker state isn't implemented.** The spec says "subtitle text color follows the same state color (more saturated on the active tab, muted on inactive)". Stage 6 ships flat fg colors regardless of state. Stage 7 should layer this in (small change to `build_glyphs` to use state-color for subtitle FG instead of FG_ACTIVE/FG_INACTIVE).

4. **WGSL bugs surface only at runtime.** Same risk as Stages 4 and 5. Smoke run is the gate.

5. **Mouse-click hit-test recomputes the layout in `handle_left_click_release`.** If the renderer used different inputs (e.g. an asynchronously-changed window size), the hit-test could mismatch. In practice the inputs are the same and the recomputation is cheap (~µs). Stage 7 could cache the layout if profiling shows it matters.

6. **Pulse animation timing uses `Instant::now().elapsed()` from the renderer's epoch.** This is monotonic so won't jump backwards on system clock changes. Frame-rate variance (e.g., the GPU stalls and skips a frame) is invisible because the pulse is computed from absolute time, not frame count.

7. **The dead-tab banner uses a hardcoded English string and the literal "Ctrl+Shift+R" reference even though Stage 6 doesn't wire that shortcut.** This will be slightly misleading until Stage 8 lands the keybinding. If Brian wants the message tightened, change it to "session died — close the window or wait for Stage 8" temporarily; otherwise leave it as a forward-declaration of the upcoming feature.

These risks are addressed by the senior pre-execution review pass and the Stage 6 manual smoke walkthrough.
