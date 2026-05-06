# vibeflow — Manual Testing Checklist

This file is the manual smoke checklist for vibeflow. Automated tests cover
the pure logic; the GUI parts require a real display and are validated by
walking through the relevant section here before tagging a stage as complete.

## Stage 4 — winit window + wgpu clear-color render plumbing

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] A window opens within ~500 ms. Title bar reads "vibeflow". Initial size
  is roughly 960×600 logical pixels.
- [ ] The window's content area is a uniform dark grey (#0e0e12). No flicker.
- [ ] Stderr shows an `INFO` line with `spawning first tab` and a `shell=`
  field reflecting your `$SHELL` (or `/bin/sh` if unset). The exact format
  is `tracing-subscriber`'s default: timestamp, level, message, fields.
- [ ] Type `echo hi` then press Enter. Stderr shows passthrough log lines
  (visible at trace level: `RUST_LOG=vibeflow=trace`). You won't see the
  output in the window yet — that's Stage 5.
- [ ] Press Ctrl+C. No crash; stderr shows the byte going through.
- [ ] Resize the window by dragging the edge. No crash; no surface errors in
  stderr; the dark grey fills the new size cleanly.
- [ ] Resize down to a tiny window (~10 px on a side). No crash; no PTY errors.
- [ ] Press Ctrl+D at an empty prompt. Stderr shows `session died`. The
  window stays open (Stage 6 will draw a dead-tab banner; Stage 4 just leaves
  the now-dead session in `App.tabs`). Click the window-manager close button
  to exit; stderr shows `close requested; exiting`, exit 0.
- [ ] Re-run; press the window-manager close button without doing anything else.
  Stderr shows `close requested; exiting`. Exit 0.
- [ ] Re-run on Wayland (if available) — the display server is selected
  automatically by winit. All checks above still pass.
- [ ] Re-run on X11 (set `WINIT_UNIX_BACKEND=x11`). All checks above still pass.

If the binary fails to start with a wgpu error, investigate GPU drivers
(`vulkaninfo`, `glxinfo`). Stage 4 hardcodes `Backends::PRIMARY` — there is
no env-var override yet. If you genuinely need GL, edit `Renderer::new` to
read `wgpu::util::backend_bits_from_env()` and set `WGPU_BACKEND=gl`.

## Stage 5 — alacritty_terminal grid + cell renderer

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] A window opens within ~500 ms. The user's shell prompt is visible inside
  it, rendered in JetBrains Mono, white-ish on the dark grey background.
- [ ] The block cursor is visible at the prompt position, with inverted colors.
- [ ] Type `echo hello world`. Each keystroke appears on screen as you type.
- [ ] Press Enter. The shell runs the command and prints `hello world` on the
  next line; the prompt re-appears below it.
- [ ] Run `ls --color`. Files appear with ANSI 16 colors (blue for directories,
  green for executables, etc.).
- [ ] Run a 256-color test: `for i in {0..255}; do printf "\033[48;5;${i}m %3d \033[0m" $i; done; echo`.
  All 256 background colors render distinctly.
- [ ] Run a truecolor test: `printf '\033[38;2;255;100;0mhello\033[0m\n'`.
  The text renders in orange (255, 100, 0).
- [ ] Resize the window. The prompt re-flows to the new width; the shell sees
  the new size (verify with `tput cols`).
- [ ] Run `vim` or `nano`. The full-screen UI renders. Cursor moves with arrow
  keys (Stage 8 actually wires arrows; for Stage 5, hjkl in vim normal mode
  works because they're letters).
- [ ] Run `clear`. Screen clears to the dark grey background, prompt at top.
- [ ] Press Ctrl+D at an empty prompt. Stderr shows `session died`. The
  rendered grid freezes at the last known state (no banner yet — that's
  Stage 6). Click the close button to exit.

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

**Known Stage 6 limitations (deferred to later stages):**

- Subtitle text isn't tinted by tracker state. Stage 7 will tint waiting
  subtitles amber, working subtitles blue, etc.
- Copy/paste is not wired (no clipboard integration yet) — Stage 8+ adds
  selection-rect rendering on mouse drag plus winit clipboard.
- Keyboard shortcuts (`Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, `Ctrl+Shift+R`)
  arrive in Stage 8.

If any step fails, capture the failure and a screenshot before fixing.

## Stage 7 — cosmic-text font shaping + subtitle tint + cursor blink + bell flash

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] Window opens within ~700 ms (slightly longer than Stage 6 because
  cosmic-text scans system fonts at startup). Tab bar at top, prompt below.
- [ ] Cursor visibly blinks at 1 Hz on the active tab.
- [ ] Type `echo "héllo wörld 中文 🎉"`. Each character renders:
  - ASCII via JetBrainsMono.
  - Latin extended (é, ö) via JBM (it has full Latin-1 coverage).
  - CJK (中文) via the user's installed CJK font (Noto Sans CJK on most
    Linux distros). If no CJK font installed: tofu boxes — that's fine.
  - Emoji (🎉) renders as monochrome outline or tofu — Stage 7.5 adds
    color emoji.
- [ ] Run `printf '\007'`. Window briefly tints white (~200 ms).
- [ ] Run `printf '\033]1338;state=waiting\007'`. Subtitle changes to
  `waiting` AND renders in **amber** (Stage 6 only changed the text;
  Stage 7 tints it).
- [ ] Run `printf '\033]1338;state=working\007'`. Subtitle in **blue**.
- [ ] Run `printf '\033]1338;state=active\007'` (or wait for it to default).
  Subtitle tint disappears (back to the default tab fg).
- [ ] Open ~10 tabs. Atlas shouldn't visibly stutter as new glyphs are
  cached. (Internal: glyph_for cache hits on repeat ASCII; misses only on
  first sighting of each codepoint.)
- [ ] Resize the window to a tiny size (~10 px). No crash; no GPU errors.
- [ ] Run an editor that hides the cursor (e.g. `vi`, then enter normal
  mode). Cursor stops blinking on the active tab while shape is Hidden.
- [ ] Press Ctrl+D in the active tab. Session dies; dead-tab banner appears
  in amber. Cursor stops blinking on the dead tab.
- [ ] Re-run with `WINIT_UNIX_BACKEND=x11`. All checks above still pass.

**Known Stage 7 limitations (deferred to later stages):**

- Color emoji renders as monochrome outline or tofu — Stage 7.5 adds the
  RGBA atlas path.
- No programming ligatures (`==>` renders as three glyphs) — Stage 8 polish.
- No bidi or complex shaping — Stage 8+.
- Font family hardcoded to JBM + system fallback — Stage 9 (TOML config).
- Cursor blink period not configurable — Stage 9.
- Bell-flash overlay always fires for the active tab; background-tab BELs
  are silently dropped (UX choice; configurable in Stage 9).

## Stage 7.5 — color emoji RGBA atlas + wide-glyph fix

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] Run `printf '🎉 🚀 😀\n'`. Each emoji renders in **full color** (not
  monochrome outline, not tofu). Some emoji may still appear as outlines
  if they only resolve to a non-color font (e.g. DejaVu Sans on Ubuntu) —
  that's a font priority issue deferred to Stage 9.
- [ ] Run `printf '中文 vs 中文\n'`. Each CJK pair renders identically; no
  overflow or overlap into adjacent cells.
- [ ] Run `printf '🎉🎉🎉\n'`. Backgrounds tile cleanly under back-to-back
  wide chars; no clipping at cell boundaries.
- [ ] Type at the prompt with cursor over a wide char. Cursor block covers
  the full 2-cell width.
- [ ] Run `for i in $(seq 1 100); do printf '%s' $(printf '\\U%x' $((0x1f600 + i % 40))); done`.
  Atlas grows; no visible stutter.
- [ ] Resize the window to ~10 px. No GPU errors; emoji still rasterized
  correctly.
- [ ] Run `vi`, enter normal mode. Cursor stops blinking on the active tab
  while shape is Hidden (Stage 7 behavior preserved).
- [ ] Press Ctrl+D in the active tab. Session dies; dead-tab banner appears
  in amber. Cursor stops blinking on the dead tab. (Stage 7 behavior.)
- [ ] On a system with NO color emoji font (uninstall Noto Color Emoji):
  emoji renders as tofu/outline. No crash.
- [ ] Re-run with `WINIT_UNIX_BACKEND=x11`. All checks above still pass.

**Known Stage 7.5 limitations (deferred to later stages):**

- No programming ligatures (`==>` renders as three glyphs) — Stage 8+.
- Cursor over a color emoji shows the emoji on a swapped background (the
  color path ignores fg/bg). Acceptable for v0.1; may revisit in Stage 9.
- No bidi or complex shaping — Stage 8+.
- Font family hardcoded to JBM + system fallback — Stage 9 (TOML config).
- Emoji font selection not configurable — many smiley-face emoji
  (U+1F600..) currently fall back to DejaVu Sans (mono) on Ubuntu rather
  than Noto Color Emoji; fontdb priority adjustment is Stage 9.

## Stage 8 — clipboard + keyboard shortcuts + selection

Run:

```bash
cd /home/bhengen/dev/vibeflow
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

- [ ] Drag-select "(base) bhengen", press `Ctrl+Shift+C`. Paste into another GUI app on slmbeast (e.g., Firefox URL bar) — should arrive as text.
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
