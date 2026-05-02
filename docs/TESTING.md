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
