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

- The cell grid still draws with row 0 at the top of the window — the topmost
  rows are clipped behind the tab bar. Stage 7+ adds a y-offset uniform to
  shift the grid down by `tab_bar_height_px`. For now, run `clear` after
  resizing to redraw the visible area.
- Subtitle text isn't tinted by tracker state. Stage 7 will tint waiting
  subtitles amber, working subtitles blue, etc.
- Keyboard shortcuts (`Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, `Ctrl+Shift+R`)
  arrive in Stage 8.

If any step fails, capture the failure and a screenshot before fixing.
