# vibeflow Stage 4 Implementation Plan: winit window + wgpu clear-color render plumbing

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open a real window with a wgpu-cleared background, drive the existing `App` from inside winit's event loop, forward keystrokes to the active tab's PTY child, propagate window resize to both the wgpu surface and the PTY, and emit `tracing` log lines for observed `SessionEvent`s. After this plan, vibeflow has a window — just no terminal content visible inside it. Stage 5 wires `alacritty_terminal` and the cell grid; Stage 6 adds the tab bar with the Notice indicator.

**Architecture:** Two new modules in the `vibeflow` crate plus extensions to `App` and `PtySession`:

- `src/render/mod.rs` — `Renderer` owns the wgpu `Instance`, `Surface`, `Device`, `Queue`, and `SurfaceConfiguration`. `new(&Window)` runs the async wgpu init via `pollster::block_on`. `render(&self)` writes a solid clear color to the surface (Stage 5 replaces this with the grid render). `resize(width, height)` reconfigures the surface.
- `src/window.rs` — `WindowApp` implements `winit::application::ApplicationHandler`. Lazy-creates the `Window` + `Renderer` in `resumed`, owns the `App`, and routes `WindowEvent`s: `Resized`, `CloseRequested`, `RedrawRequested`, `KeyboardInput`. Keystrokes are translated to bytes by a small private helper and pushed via `app.send_input`. In `about_to_wait` it drains `app.poll_all` + `app.tick_all`, logs each `SessionEvent` via `tracing`, and sets `ControlFlow::WaitUntil(now + 100ms)` so trackers tick at ~10 Hz.
- `src/main.rs` (rewrite) — initialises `tracing-subscriber`, builds an `EventLoop`, calls `event_loop.run_app(&mut WindowApp::new())`. Replaces the Stage 3 sleep-loop demo entirely.
- `src/session/session.rs` (modify) — `PtySession` keeps the `master` handle on the main thread (the Stage 3 reader-thread closure consumed it; we move it back) so `resize(rows, cols)` can call `master.resize(...)`. The reader thread no longer needs the `_master_alive` binding because the main thread keeps the master alive for the lifetime of the session.
- `src/app.rs` (modify) — adds `App::resize_all(rows, cols)` that fans out to every session.

The threading model from Stages 1–3 stays exact: the reader thread blocks on `read()` and sends bytes via `mpsc`; the main thread (now winit's loop) mutates all state. `Renderer` lives on the main thread. No tokio. `pollster::block_on` is used only for the small synchronous wgpu init code where wgpu's API surface is async-typed but conceptually instantaneous.

**Tech Stack:** Adds:

```toml
winit = "0.30"
wgpu = "0.20"
pollster = "0.3"     # block_on for wgpu's async-typed but sync-in-practice init
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

`portable-pty`, `vibeflow-protocol`, and `proptest` carry over from prior stages.

**Stage scope:** Stage 4 ends with a runnable `vibeflow` that opens a window, accepts a useful subset of keyboard input (printable chars + Enter + Backspace + Ctrl+C + Ctrl+D), resizes the PTY along with the window, and logs `SessionEvent`s. The window content itself is just a solid Stage-4 placeholder color (`#0e0e12` per the spec's default theme). **It is not yet a usable terminal** — that lands in Stage 5 when `alacritty_terminal` and the cell-grid renderer arrive. The acceptance criterion for Stage 4 is: type `echo hi\n`, observe the bytes in the `tracing` log, observe the shell echoing them back, observe a `StateChanged` event when the prompt re-emits OSC 133, and close the window cleanly.

**Lessons carried forward from Stages 1–3:**
- Pre-fmt the verbatim Rust code (rustfmt prefers wider line breaking than the human-readable plan style).
- Forward-declared items get `#[allow(dead_code)]` until the first lib-level caller arrives, with cleanup in the introducing-caller task.
- Per-task `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings` verify step before commit.
- Intra-doc links must reference items in scope (`[`Self::method`]`, not `[`method`]`) to satisfy `RUSTDOCFLAGS="-D warnings" cargo doc`.
- For tests that depend on subprocess byte emission, prefer `python3 -c "import sys; sys.stdout.buffer.write(bytes([...]))"` over `/bin/sh -c "printf '\xNN...'"` — Ubuntu's `/bin/sh` is `dash`, whose `printf` does not interpret `\xNN` hex escapes (only `\NNN` octal). For static byte strings inside shell `printf`, octal works.
- winit + wgpu cannot be unit-tested headlessly in CI without a display server. This stage's tests cover only the pure logic (resize math, keystroke encoding); the GUI parts are validated by the manual smoke checklist in Task 9.

**A note on testing this stage:** Unlike Stages 1–3 which used aggressive TDD throughout, the GUI plumbing in `window.rs` and `render/mod.rs` is imperative driver code that interacts with hardware (display server, GPU). It cannot meaningfully be unit-tested. Tasks 1, 6, and 7 use TDD for the pure logic they introduce (PTY resize delegation, pixel-to-cell math, key-to-bytes encoding). Tasks 2, 3, 4, 5, 8 are imperative implementation tasks verified by `cargo build`, `cargo clippy`, and the smoke checklist.

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Add `winit = "0.30"`, `wgpu = "0.20"`, `pollster = "0.3"`, `anyhow = "1"`, `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`. |
| `crates/vibeflow/src/lib.rs` (modify) | Declare `pub mod render;` and `pub mod window;`. |
| `crates/vibeflow/src/render/mod.rs` (new) | `Renderer` struct: wgpu instance, surface, device, queue, surface_config. `new`, `resize`, `render` (clear-color). ~150 LOC. |
| `crates/vibeflow/src/window.rs` (new) | `WindowApp` implementing `winit::application::ApplicationHandler`. Owns `Option<Arc<Window>>`, `Option<Renderer>`, `App`. ~280 LOC. |
| `crates/vibeflow/src/main.rs` (rewrite) | `tracing_subscriber::fmt()` init + `EventLoop::new()? .run_app(&mut WindowApp::new())`. ~30 LOC. |
| `crates/vibeflow/src/session/session.rs` (modify) | Keep `master` field; add `pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()>`. Drop the `_master_alive` binding from the reader-thread closure. |
| `crates/vibeflow/src/app.rs` (modify) | Add `pub fn resize_all(&self, rows: u16, cols: u16) -> io::Result<()>`. |
| `docs/TESTING.md` (new or extend) | Manual smoke checklist for Stage 4. |

---

## Task 0: Add deps + module declarations + stubs

**Files:**
- Modify: `crates/vibeflow/Cargo.toml`
- Modify: `crates/vibeflow/src/lib.rs`
- Create: stubs for `crates/vibeflow/src/render/mod.rs`, `crates/vibeflow/src/window.rs`

- [ ] **Step 1: Add the new dependencies**

Edit `crates/vibeflow/Cargo.toml`. The current `[dependencies]` section (after Stage 3) is:

```toml
[dependencies]
vibeflow-protocol = { path = "../vibeflow-protocol", version = "0.1" }
portable-pty = "0.8"
```

Replace it with:

```toml
[dependencies]
vibeflow-protocol = { path = "../vibeflow-protocol", version = "0.1" }
portable-pty = "0.8"
winit = "0.30"
wgpu = "0.20"
pollster = "0.3"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

Leave `[package]`, `[lints]`, `[lib]`, `[[bin]]`, `[dev-dependencies]` unchanged.

**Why these versions:** the design spec pins them; they're the locked-in set as of design time. winit 0.30 introduced the `ApplicationHandler` trait — the entire event-loop pattern this plan uses depends on that version or later. wgpu 0.20 matches alacritty's current set. `pollster` is a tiny zero-dep crate that just runs a future to completion synchronously.

- [ ] **Step 2: Declare the new modules in `lib.rs`**

Edit `crates/vibeflow/src/lib.rs`. Replace the contents with:

```rust
//! `vibeflow` — GPU-accelerated terminal emulator for Linux that signals AI-tool state.
//!
//! Stage 4 of v0.1 adds the winit window and the wgpu render pipeline. The
//! visible content is still just a solid clear color — the cell-grid renderer
//! arrives in Stage 5. Public surface: [`session`], [`app`], [`render`], [`window`].
//!
//! See `docs/superpowers/specs/2026-05-01-vibeflow-design.md` for the full design.

pub mod app;
pub mod render;
pub mod session;
pub mod window;
```

- [ ] **Step 3: Stub the new files**

Write `crates/vibeflow/src/render/mod.rs`:

```rust
//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.
```

Write `crates/vibeflow/src/window.rs`:

```rust
//! winit `ApplicationHandler` integration: the [`WindowApp`] type owns the
//! `Window`, the [`crate::render::Renderer`], and the [`crate::app::App`].
//! Drives polling, ticking, and event routing on the main thread.
```

- [ ] **Step 4: Verify the workspace builds**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build (the new modules are stubs but valid). Clippy silent. `Cargo.lock` will have grown — winit, wgpu, and their transitive deps are added.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add Cargo.toml crates/vibeflow/Cargo.toml crates/vibeflow/src/lib.rs crates/vibeflow/src/render/mod.rs crates/vibeflow/src/window.rs Cargo.lock
git commit -m "chore(vibeflow): add winit/wgpu deps and module stubs for Stage 4"
```

---

## Task 1: PtySession::resize + App::resize_all (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`
- Modify: `crates/vibeflow/src/app.rs`

`MasterPty::resize` from `portable-pty` takes `&self` (interior-mutable). To call it we need to keep the master on the main thread instead of moving it into the reader thread. The Stage 3 closure binds it as `_master_alive` only to keep the PTY alive while reads happen; that responsibility moves to `PtySession`'s field.

This is a TDD task: we write the resize call, then verify it succeeds (we trust portable-pty's tests for the actual ioctl-level semantics — verifying `stty size` from inside a test child process is brittle and timing-dependent).

- [ ] **Step 1: Write the failing test (resize delegation)**

Append to the existing `mod tests` block in `crates/vibeflow/src/session/session.rs`:

```rust
    #[test]
    fn resize_does_not_error_on_a_live_session() {
        // We don't assert anything about the child observing the new size — the
        // ioctl semantics are portable-pty's responsibility. We just verify the
        // call succeeds end-to-end (no Mutex poisoning, no consumed-master
        // panic, no Result::Err path).
        let s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
        s.resize(40, 100).unwrap();
        // Issue a second resize to verify it's not a one-shot.
        s.resize(24, 80).unwrap();
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: compile error — `resize` not defined on `PtySession`.

- [ ] **Step 2: Refactor `PtySession::spawn` to keep `master` on the main thread**

In `crates/vibeflow/src/session/session.rs`, change the struct to add a `master` field and update `spawn` so the reader thread no longer captures the master.

Change the struct definition (currently has 7 fields):

```rust
pub struct PtySession {
    /// Drains here when the reader thread sends bytes from the PTY master.
    rx: Receiver<Vec<u8>>,
    /// Used by [`Self::send_input`] to write keystrokes to the PTY master.
    writer: Box<dyn Write + Send>,
    /// The PTY master. Kept alive on the main thread; the reader thread holds a
    /// cloned `Box<dyn Read + Send>` whose lifetime is independent of this
    /// field. `MasterPty::resize` is called through this handle.
    master: Box<dyn portable_pty::MasterPty + Send>,
    /// Child process handle — used for liveness checks and explicit kill.
    child: Box<dyn Child + Send + Sync>,
    /// Reader thread handle. Owned by the session; joined when `Drop` runs.
    reader_thread: Option<JoinHandle<()>>,
    /// Per-session OSC parser.
    dispatcher: OscDispatcher,
    /// Per-session state tracker.
    tracker: AiStateTracker,
    /// True until either the child exits or the reader-thread errors out.
    alive: bool,
}
```

Then change `PtySession::spawn`. The current Stage 3 body destructures `PtyHandles` and moves `master` into the reader-thread closure as `_master_alive`. Replace it with:

```rust
    pub fn spawn(argv: &[&str], config: TrackerConfig) -> std::io::Result<Self> {
        let PtyHandles {
            reader,
            writer,
            child,
            master,
        } = spawn_pty(argv)?;
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut reader = reader;
        let reader_thread = thread::Builder::new()
            .name("vibeflow-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            rx,
            writer,
            master,
            child,
            reader_thread: Some(reader_thread),
            dispatcher: OscDispatcher::new(),
            tracker: AiStateTracker::new(config),
            alive: true,
        })
    }
```

The only structural changes from Stage 3 are: (1) `master` is added to the destructured local and to the returned struct; (2) the `let _master_alive = master;` line inside the closure is gone. The reader thread's `Box<dyn Read + Send>` (built earlier from `master.try_clone_reader()`) keeps reading until the child closes its end of the PTY, at which point read returns `Ok(0)` and the thread exits. The master stays alive on the main thread for the entire `PtySession` lifetime.

- [ ] **Step 3: Implement `PtySession::resize`**

Add the following method to `impl PtySession`, anywhere within the impl block — convention is to keep it grouped with `send_input` and the other public actions, so place it after `tick`:

```rust
    /// Resize the PTY to `rows` rows × `cols` cols. The kernel sends `SIGWINCH`
    /// to the foreground process group so well-behaved children re-render.
    ///
    /// `pixel_width` / `pixel_height` are reported as 0 — most consumers ignore
    /// them; pixel dimensions matter only to image protocols (sixel, kitty
    /// graphics) which v0.1 doesn't implement.
    ///
    /// # Errors
    /// Wraps `portable_pty`'s typed error via `io::Error::other`.
    pub fn resize(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        self.master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)
    }
```

Note that `resize` takes `&self` not `&mut self` because `MasterPty::resize` takes `&self` (it's interior-mutable — POSIX `ioctl(TIOCSWINSZ)` mutates kernel state, not the master handle).

- [ ] **Step 4: Run the test**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: 8 tests pass (7 prior + 1 new).

- [ ] **Step 5: Implement `App::resize_all` (TDD)**

Append to the existing `mod tests` block in `crates/vibeflow/src/app.rs`:

```rust
    #[test]
    fn resize_all_fans_out_to_every_session() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        // Expect Ok and no panic. Real per-tab observation lives in the
        // PtySession-level test in session::session.
        app.resize_all(40, 100).unwrap();
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib app
```

Expected: compile error — `resize_all` not defined on `App`.

- [ ] **Step 6: Implement `App::resize_all`**

Add the following to `impl App`, after `send_input`:

```rust
    /// Resize every tab's PTY to `rows × cols`. Called from `WindowApp` on
    /// every `WindowEvent::Resized` after the renderer surface is reconfigured.
    ///
    /// # Errors
    /// Returns the first per-tab `io::Error`; subsequent tabs are still resized
    /// best-effort. (We bias to applying the resize as broadly as possible
    /// because a single tab's resize failure shouldn't block the others — but
    /// we still surface the error so the caller can log it.)
    pub fn resize_all(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        let mut first_error: Option<std::io::Error> = None;
        for tab in &self.tabs {
            if let Err(e) = tab.resize(rows, cols) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        match first_error {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
```

`resize_all` takes `&self` (not `&mut`) because `PtySession::resize` is `&self`.

- [ ] **Step 7: Verify all tests + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: all prior tests + 2 new pass; fmt + clippy silent.

- [ ] **Step 8: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/session.rs crates/vibeflow/src/app.rs
git commit -m "feat(session,app): PtySession::resize and App::resize_all"
```

---

## Task 2: Renderer with wgpu init + clear-color render

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

This task introduces the `Renderer` type. It has no unit tests — wgpu init talks to a real GPU/driver and can't be exercised meaningfully in headless CI. Verification is by `cargo build` (compile-time correctness) and the manual smoke run in Task 9.

- [ ] **Step 1: Replace the contents of `crates/vibeflow/src/render/mod.rs`**

```rust
//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::window::Window;

/// Default clear color for Stage 4 — matches the dark-theme background from
/// `docs/superpowers/specs/2026-05-01-vibeflow-design.md` (`#0e0e12`).
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0x0e as f64 / 255.0,
    g: 0x0e as f64 / 255.0,
    b: 0x12 as f64 / 255.0,
    a: 1.0,
};

/// All wgpu state that lives for the duration of the window. Created once in
/// [`Renderer::new`] and dropped when the window closes.
///
/// The `Surface` borrows from the `Window`; we hold an `Arc<Window>` so the
/// lifetime is tied to the renderer rather than the calling scope.
pub struct Renderer {
    /// Kept so the surface's borrow stays valid for the renderer's lifetime.
    _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
}

impl Renderer {
    /// Initialise wgpu against the given window. Blocks on the few async wgpu
    /// calls via [`pollster::block_on`]; the operations are conceptually
    /// instantaneous (no I/O), they're just async-typed for tokio compatibility.
    ///
    /// # Errors
    /// Any wgpu init step that fails — instance creation, surface creation,
    /// adapter request (no compatible GPU), device request, surface
    /// configuration. Each error is wrapped with a `context()` describing the
    /// failed step.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        // Width/height of zero is invalid for surface configuration. winit may
        // hand us a (0, 0) on the very first frame on some compositors;
        // clamp to 1 so the surface configures successfully.
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // SAFETY: `Surface<'static>` requires the surface target to live as
        // long as the surface. We hold an `Arc<Window>` in the returned struct,
        // so the window outlives the surface.
        let surface = instance
            .create_surface(window.clone())
            .context("create wgpu surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("no compatible GPU adapter found (try VIBEFLOW_BACKEND=gl)")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("vibeflow-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .context("request wgpu device + queue")?;

        let surface_caps = surface.get_capabilities(&adapter);
        // Prefer sRGB so colours match designer expectations; fall back to the
        // first format if no sRGB option is offered.
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            _window: window,
            surface,
            device,
            queue,
            surface_config,
        })
    }

    /// Reconfigure the surface for a new physical size. `winit::WindowEvent::Resized`
    /// fires this; the new dimensions are the *physical* (post-DPI-scaling) pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.surface_config.width == width && self.surface_config.height == height {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Submit a single-frame render of the clear color. Stage 5 replaces the
    /// body of this method with the cell-grid render; the public signature
    /// stays the same.
    pub fn render(&self) -> std::result::Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vibeflow-frame-encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vibeflow-clear-pass"),
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
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }

    /// Re-apply the current `surface_config`. Used to recover from
    /// `SurfaceError::Lost` / `Outdated` — those errors mean the surface needs
    /// to be re-created with its current settings.
    pub fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Current surface width/height in physical pixels. Stage 4's resize math
    /// uses these to compute terminal cell rows/cols.
    #[must_use]
    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }
}
```

- [ ] **Step 2: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build. Clippy may emit no-op lints because `Renderer` has no callers yet — Task 3 introduces them. If clippy fires `dead_code` on `Renderer`, add `#[allow(dead_code)]` to the struct with a comment explaining the first lib-level user is `WindowApp` in Task 3 — narrow it to fields if possible, otherwise to the whole struct.

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(render): wgpu Renderer with clear-color frame and resize"
```

---

## Task 3: WindowApp skeleton with winit ApplicationHandler

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

This task introduces `WindowApp` and gives it a working `resumed` handler that opens a window, initialises the `Renderer`, and spawns the first tab. The `window_event` handler covers `CloseRequested` (exit) and `RedrawRequested` (renderer.render). Resize, keyboard, and `about_to_wait` arrive in Tasks 5–7.

- [ ] **Step 1: Replace the contents of `crates/vibeflow/src/window.rs`**

```rust
//! winit `ApplicationHandler` integration: the [`WindowApp`] type owns the
//! `Window`, the [`crate::render::Renderer`], and the [`crate::app::App`].
//! Drives polling, ticking, and event routing on the main thread.

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::app::App;
use crate::render::Renderer;

/// User-facing winit application. Implements [`ApplicationHandler`].
///
/// Lifecycle:
/// - `new()` builds an empty `App` (no tabs yet) — no winit calls.
/// - `resumed()` (called once after `event_loop.run_app`) creates the window,
///   initialises the renderer, and spawns the first tab.
/// - `window_event(...)` routes user events (close, redraw, resize, keys).
/// - `about_to_wait(...)` (added in Task 5) drains `App::poll_all` and
///   `App::tick_all`, logs `SessionEvent`s via tracing, schedules the next
///   wake-up via `ControlFlow::WaitUntil`.
pub struct WindowApp {
    /// `None` until `resumed` fires for the first time. After that, `Some` for
    /// the entire process lifetime.
    window: Option<Arc<Window>>,
    /// `None` until window is created and wgpu init succeeds. Wrapped in an
    /// `Option` so `resumed` can construct it lazily.
    renderer: Option<Renderer>,
    /// The application core. Holds every tab.
    app: App,
}

impl WindowApp {
    /// Build a `WindowApp` with no window and no tabs. Call
    /// `event_loop.run_app(&mut app)` to drive it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            app: App::new(),
        }
    }

    /// Spawn the user's shell as the first tab. `$SHELL` if set; otherwise
    /// `/bin/sh`. The path is logged at info level.
    fn spawn_first_tab(&mut self) -> Result<()> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        tracing::info!(shell = %shell, "spawning first tab");
        self.app
            .new_tab(&[shell.as_str()])
            .with_context(|| format!("spawn first tab via {shell}"))?;
        Ok(())
    }
}

impl Default for WindowApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // resumed can fire more than once on some platforms (e.g. when the
            // app comes back from suspended). For Stage 4 we only construct the
            // window the first time.
            return;
        }
        let window_attrs = Window::default_attributes()
            .with_title("vibeflow")
            .with_inner_size(winit::dpi::LogicalSize::new(960, 600));
        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!(error = %e, "failed to create window");
                event_loop.exit();
                return;
            }
        };
        let renderer = match Renderer::new(window.clone()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = ?e, "failed to initialise renderer");
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window);
        self.renderer = Some(renderer);

        if let Err(e) = self.spawn_first_tab() {
            tracing::error!(error = ?e, "failed to spawn first tab");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("close requested; exiting");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_ref() {
                    if let Err(e) = renderer.render() {
                        tracing::warn!(error = ?e, "render error");
                    }
                }
            }
            // Resize, KeyboardInput, etc. arrive in Tasks 6–7.
            _ => {}
        }
    }
}
```

- [ ] **Step 2: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build. The `Renderer` `dead_code` allow from Task 2 should now drop because `WindowApp::resumed` constructs and uses one. Remove the `#[allow(dead_code)]` on `Renderer` if you added it in Task 2's Step 2.

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs crates/vibeflow/src/render/mod.rs
git commit -m "feat(window): WindowApp with winit ApplicationHandler skeleton"
```

(Include `render/mod.rs` only if you removed an `#[allow(dead_code)]` annotation from it.)

---

## Task 4: main.rs winit EventLoop entry + tracing init

**Files:**
- Modify: `crates/vibeflow/src/main.rs`

The Stage 3 `main.rs` was a sleep-loop demo. Replace it with the winit entry point. tracing-subscriber is initialised here too — it's a process-global resource, and `WindowApp` already uses `tracing::info!`.

- [ ] **Step 1: Replace the contents of `crates/vibeflow/src/main.rs`**

```rust
//! `vibeflow` binary entry point.
//!
//! Initialises tracing, builds a winit `EventLoop`, and runs the
//! [`vibeflow::window::WindowApp`]. The Stage 3 sleep-loop demo is gone —
//! Stage 4 is the real binary.

use anyhow::{Context, Result};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use vibeflow::window::WindowApp;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> Result<()> {
    // tracing-subscriber: respect RUST_LOG when set, otherwise default to
    // `vibeflow=info,warn`. Stage 9 will add file logging + rotation.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("vibeflow=info,warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();

    let event_loop = EventLoop::new().context("create winit EventLoop")?;
    // `Wait` is the cheapest mode — the loop blocks until an event arrives.
    // Task 5 swaps this for `WaitUntil(deadline)` so tracker timeouts fire.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = WindowApp::new();
    event_loop.run_app(&mut app).context("run winit event loop")?;
    Ok(())
}
```

- [ ] **Step 2: Build the binary**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
```

Expected: clean build. The binary is now a real GUI app — running it will open an empty window.

- [ ] **Step 3: Smoke-run (manual)**

```bash
cd /home/bhengen/dev/vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

Expected behavior:
- A window opens, ~960×600, titled "vibeflow", showing a dark grey background (`#0e0e12`).
- `[INFO  vibeflow::window] spawning first tab` appears in stderr.
- Pressing the close button (or window manager `x`) exits the binary cleanly. Stderr shows `[INFO  vibeflow::window] close requested; exiting`.
- Keystrokes do nothing visible yet (Task 7 wires them).
- Resize does nothing visible yet (Task 6 wires it).

If the window doesn't open: check `$DISPLAY` / `$WAYLAND_DISPLAY` — winit needs one. If wgpu init fails, the binary exits with `failed to initialise renderer` in stderr; try `WGPU_BACKEND=gl` (winit reads this env var to pick the GL backend) on minimal GPU drivers.

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/main.rs
git commit -m "feat(vibeflow): winit EventLoop entry + tracing-subscriber init"
```

---

## Task 5: Wire App into the event loop (poll_all + tick_all + ControlFlow::WaitUntil)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

`about_to_wait` is winit's "I'm about to block waiting for events" callback. It's the natural place to drain mpsc events from the PTY readers (`App::poll_all`) and to fire tracker timeouts (`App::tick_all`). For Stage 4 we use a fixed 100ms wake-up cadence so timeouts fire reliably without computing exact deadlines (that optimisation lands in Stage 6+ when there's pressure on idle CPU).

State changes log via `tracing::info!`. `Died` events log at `warn`. `PassThrough` bytes are dropped on the floor for Stage 4 (Stage 5 wires them into `alacritty_terminal`); they're noisy at high throughput, so they go through at `trace` level only.

- [ ] **Step 1: Add the `about_to_wait` impl + the event-handling helper**

In `crates/vibeflow/src/window.rs`, add the following imports near the top (after the existing `use` lines):

```rust
use std::time::{Duration, Instant};

use winit::event_loop::ControlFlow;

use crate::session::SessionEvent;
```

Note: `Instant` was previously not in this file; it's needed for the wake-up deadline.

Inside `impl ApplicationHandler for WindowApp`, after the `window_event` method, add:

```rust
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        // Drain bytes that arrived since last tick.
        for (idx, ev) in self.app.poll_all(now) {
            self.handle_session_event(idx, ev);
        }
        // Fire any timeout-driven transitions.
        for (idx, ev) in self.app.tick_all(now) {
            self.handle_session_event(idx, ev);
        }

        // Re-arm a 100ms wake-up so trackers tick steadily. Stage 6+ will
        // compute the exact next deadline from the per-session tracker state.
        event_loop
            .set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(100)));

        // No tabs left → exit cleanly.
        if self.app.tabs().is_empty() {
            tracing::info!("all tabs closed; exiting");
            event_loop.exit();
        }
    }
```

Outside the trait impl (i.e. in `impl WindowApp`), add the helper method that handles each `SessionEvent`. Insert it after `spawn_first_tab`:

```rust
    /// React to a single `SessionEvent`. Stage 4 just logs; Stage 5 will route
    /// `PassThrough` bytes into the per-tab `alacritty_terminal` grid and call
    /// `window.request_redraw()` on `StateChanged`.
    fn handle_session_event(&mut self, idx: usize, ev: SessionEvent) {
        match ev {
            SessionEvent::StateChanged(state) => {
                tracing::info!(tab = idx, state = ?state, "state changed");
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            SessionEvent::PassThrough(bytes) => {
                // Stage 5 sends these into alacritty_terminal::Term::input.
                // Stage 4 just records the byte count at trace level so we can
                // sanity-check throughput from the log without spamming.
                tracing::trace!(tab = idx, bytes = bytes.len(), "passthrough");
            }
            SessionEvent::Died => {
                tracing::warn!(tab = idx, "session died");
                // Stage 6 (tab bar) renders the dead-tab banner. For now we
                // just close the tab so `about_to_wait` will exit if it was
                // the last one.
                self.app.close_tab(idx);
            }
        }
    }
```

- [ ] **Step 2: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 3: Smoke-run (manual)**

```bash
cd /home/bhengen/dev/vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

Expected:
- Window opens; first tab's shell prompt is rendered to the PTY but invisible (no grid yet).
- Stderr shows `state changed` lines as the prompt's OSC 133 hooks fire (only if your shell is configured with the OSC 133 PS1 hook from `shells/`; otherwise the tracker sees no events and stays Active).
- After ~100ms idle, a `state changed` to `Idle` may appear if your shell emits prompt markers.
- Closing the window exits cleanly with `close requested; exiting`.

If the shell doesn't emit OSC 133 (default zsh/bash without our PS1 hook), no state changes will be logged. That's expected for Stage 4 — Stages 6+ ship the shell hook scripts.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): drive App::poll_all and App::tick_all from about_to_wait"
```

---

## Task 6: Window resize → renderer + PTY (TDD on the pure math)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

Window resize touches three things: the wgpu surface (renderer), the PTY (so the foreground process can re-layout), and a future redraw. The pixel-to-cell math is pure logic — the `pixels_to_grid` helper has unit tests.

For Stage 4 we hardcode a placeholder cell size of 8×16 px. Stage 7 (font atlas) replaces this with values derived from cosmic-text font metrics. Until then, a 960×600 window gives ~120 cols × 37 rows.

- [ ] **Step 1: Write the failing unit test for `pixels_to_grid`**

Add a test module to `crates/vibeflow/src/window.rs`. After all the existing code, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_to_grid_uses_floor_division() {
        // 960 / 8 = 120, 600 / 16 = 37 (with 8 px remainder ignored).
        assert_eq!(pixels_to_grid(960, 600, 8, 16), (37, 120));
    }

    #[test]
    fn pixels_to_grid_clamps_to_minimum_one_cell() {
        // A degenerate 0×0 surface should still produce 1×1 — terminal
        // children expect at least one row and one column.
        assert_eq!(pixels_to_grid(0, 0, 8, 16), (1, 1));
    }

    #[test]
    fn pixels_to_grid_handles_unusual_cell_sizes() {
        // Square cells, 100×100 surface.
        assert_eq!(pixels_to_grid(100, 100, 10, 10), (10, 10));
    }
}
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib window
```

Expected: compile error — `pixels_to_grid` not defined.

- [ ] **Step 2: Implement `pixels_to_grid` and the resize event handler**

In `crates/vibeflow/src/window.rs`, near the top of the file (above `pub struct WindowApp`), add:

```rust
/// Stage-4 placeholder cell size. Stage 7 (font atlas) replaces this with
/// values derived from cosmic-text font metrics for the configured font.
const CELL_WIDTH_PX: u32 = 8;
const CELL_HEIGHT_PX: u32 = 16;

/// Compute terminal grid dimensions (rows, cols) from a window's physical
/// pixel size and per-cell pixel size. Floor-divides; clamps to at least 1×1
/// so degenerate (0, 0) surfaces still produce a usable grid for the child.
fn pixels_to_grid(width_px: u32, height_px: u32, cell_w: u32, cell_h: u32) -> (u16, u16) {
    let cols = (width_px / cell_w).max(1);
    let rows = (height_px / cell_h).max(1);
    // PTY size fields are u16. Realistic terminal sizes are well under
    // u16::MAX (~65k cells), but we still saturate-cast defensively.
    (rows.min(u16::MAX as u32) as u16, cols.min(u16::MAX as u32) as u16)
}
```

In the `window_event` match arm, add a `Resized` arm above the catch-all `_ => {}`:

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

- [ ] **Step 3: Run the unit tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib window
```

Expected: 3 unit tests pass.

- [ ] **Step 4: Verify fmt + clippy + smoke**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

Manually drag the window edge to resize. Expected: no panics, no errors in stderr, PTY children re-layout (you can verify by running `clear; tput cols; tput lines` in the shell tab once Stage 5 ships visible output — for Stage 4 it's a non-visible smoke test).

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): propagate WindowEvent::Resized to wgpu surface and PTY"
```

---

## Task 7: Keyboard input forwarding (TDD on the encoding)

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

Stage 4 ships a minimal keystroke-to-bytes encoder: printable Unicode characters, Enter (`\r`), Backspace (`0x7f`), Ctrl+C (`0x03`), Ctrl+D (`0x04`). Anything else falls through to `None` (ignored). Stage 8 layers in arrows, function keys, full modifier support.

The encoding logic is a pure function — `key_to_bytes(&KeyEvent) -> Option<Vec<u8>>` — so it's TDD-friendly.

- [ ] **Step 1: Write the failing unit tests for `key_to_bytes`**

Append to the existing `mod tests` block in `crates/vibeflow/src/window.rs`:

```rust
    use winit::event::{ElementState, KeyEvent};
    use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

    fn key_press(logical: Key, modifiers: ModifiersState) -> KeyEvent {
        // winit::event::KeyEvent has a `repeat` field and platform-specific
        // bits. We construct only the minimum the encoder reads.
        KeyEvent {
            physical_key: winit::keyboard::PhysicalKey::Unidentified(
                winit::keyboard::NativeKeyCode::Unidentified,
            ),
            logical_key: logical,
            text: None,
            location: winit::keyboard::KeyLocation::Standard,
            state: ElementState::Pressed,
            repeat: false,
            platform_specific: Default::default(),
            modifiers,
        }
    }

    #[test]
    fn key_to_bytes_printable_ascii() {
        let ev = key_press(Key::Character(SmolStr::new("a")), ModifiersState::empty());
        assert_eq!(key_to_bytes(&ev), Some(b"a".to_vec()));
    }

    #[test]
    fn key_to_bytes_printable_unicode() {
        let ev = key_press(Key::Character(SmolStr::new("é")), ModifiersState::empty());
        assert_eq!(key_to_bytes(&ev), Some("é".as_bytes().to_vec()));
    }

    #[test]
    fn key_to_bytes_enter_returns_carriage_return() {
        let ev = key_press(Key::Named(NamedKey::Enter), ModifiersState::empty());
        assert_eq!(key_to_bytes(&ev), Some(vec![b'\r']));
    }

    #[test]
    fn key_to_bytes_backspace_returns_del() {
        let ev = key_press(Key::Named(NamedKey::Backspace), ModifiersState::empty());
        assert_eq!(key_to_bytes(&ev), Some(vec![0x7f]));
    }

    #[test]
    fn key_to_bytes_ctrl_c_returns_etx() {
        let ev = key_press(Key::Character(SmolStr::new("c")), ModifiersState::CONTROL);
        assert_eq!(key_to_bytes(&ev), Some(vec![0x03]));
    }

    #[test]
    fn key_to_bytes_ctrl_d_returns_eot() {
        let ev = key_press(Key::Character(SmolStr::new("d")), ModifiersState::CONTROL);
        assert_eq!(key_to_bytes(&ev), Some(vec![0x04]));
    }

    #[test]
    fn key_to_bytes_ignores_release_events() {
        let mut ev = key_press(Key::Character(SmolStr::new("a")), ModifiersState::empty());
        ev.state = ElementState::Released;
        assert_eq!(key_to_bytes(&ev), None);
    }

    #[test]
    fn key_to_bytes_ignores_unhandled_named_keys() {
        // F5 is not in Stage 4's subset; Stage 8 will handle it.
        let ev = key_press(Key::Named(NamedKey::F5), ModifiersState::empty());
        assert_eq!(key_to_bytes(&ev), None);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib window
```

Expected: compile error — `key_to_bytes` not defined.

- [ ] **Step 2: Implement `key_to_bytes` and wire `WindowEvent::KeyboardInput`**

In `crates/vibeflow/src/window.rs`, add the following imports near the existing `use` lines:

```rust
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};
```

Above the `pub struct WindowApp` declaration (next to `pixels_to_grid` from Task 6), add:

```rust
/// Translate a winit key-press event into the bytes the PTY child expects on
/// stdin. Returns `None` for releases, modifier-only events, and any key not
/// in Stage 4's minimal subset (Stage 8 fills in arrows, F-keys, full Alt/Meta
/// handling, etc.).
fn key_to_bytes(event: &KeyEvent) -> Option<Vec<u8>> {
    if event.state != ElementState::Pressed {
        return None;
    }
    match &event.logical_key {
        Key::Character(s) => {
            if event.modifiers.contains(ModifiersState::CONTROL) {
                // Ctrl+letter → 0x01..=0x1A. Only handle a..=z; leave numbers
                // and punctuation to Stage 8.
                let lower = s.to_ascii_lowercase();
                let mut chars = lower.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if c.is_ascii_lowercase() {
                        return Some(vec![(c as u8) - b'`']); // 'a' (0x61) - 0x60 = 0x01
                    }
                }
                None
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        Key::Named(NamedKey::Enter) => Some(vec![b'\r']),
        Key::Named(NamedKey::Backspace) => Some(vec![0x7f]),
        Key::Named(NamedKey::Tab) => Some(vec![b'\t']),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        // Anything else → Stage 8.
        _ => None,
    }
}
```

In the `window_event` match arm in `impl ApplicationHandler for WindowApp`, add a `KeyboardInput` arm above the catch-all:

```rust
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(bytes) = key_to_bytes(&event) {
                    if let Err(e) = self.app.send_input(&bytes) {
                        tracing::warn!(error = %e, "send_input failed");
                    }
                }
            }
```

- [ ] **Step 3: Run the unit tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib window
```

Expected: 11 tests pass (3 from Task 6 + 8 new from this task).

- [ ] **Step 4: Verify fmt + clippy + smoke**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
cargo build --bin vibeflow
RUST_LOG=vibeflow=info,trace ./target/debug/vibeflow
```

In the running window, type:
- Letters: e.g. `echo hi`. Each keystroke writes a single byte to the PTY.
- Enter: triggers the shell to run the command. (You won't see the output yet — Stage 5 wires the grid.)
- Stderr should show `passthrough` trace lines once the shell echoes back the typed bytes and prints "hi\n".
- Ctrl+C: should send SIGINT to the foreground process (no visible effect in Stage 4 since the shell is just sitting at a prompt).
- Ctrl+D at an empty prompt: shell exits, you should see `state changed` lines and eventually `session died` followed by `all tabs closed; exiting`.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): forward keystrokes to active tab via App::send_input"
```

---

## Task 8: Surface error recovery

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

`Renderer::render` returns `Result<(), wgpu::SurfaceError>`. The Stage 3 handler in Task 3's `RedrawRequested` arm just `tracing::warn`s and drops the error. Stage 4 needs to recover from `Lost` and `Outdated` (reconfigure the surface) and panic on `OutOfMemory` (per the spec's "fail loud at startup, fail soft at runtime" — OOM is an unrecoverable runtime failure but it's vanishingly rare in practice).

- [ ] **Step 1: Replace the `RedrawRequested` arm**

In `crates/vibeflow/src/window.rs`, locate the existing arm:

```rust
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_ref() {
                    if let Err(e) = renderer.render() {
                        tracing::warn!(error = ?e, "render error");
                    }
                }
            }
```

Replace the body with the surface-error-aware version:

```rust
            WindowEvent::RedrawRequested => {
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                match renderer.render() {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        // Surface needs to be re-created with current config.
                        // The existing render config (size, format) is reused.
                        renderer.reconfigure();
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        tracing::error!("GPU out of memory; exiting");
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {
                        // Frame took longer than the deadline. Skip this frame
                        // and request another. Common during driver hiccups.
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
```

`renderer.as_mut()` (instead of `as_ref()`) is needed because `reconfigure` takes `&mut self`.

- [ ] **Step 2: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): recover from SurfaceError::Lost/Outdated/Timeout"
```

---

## Task 9: Manual smoke checklist

**Files:**
- Create or extend: `docs/TESTING.md`

The spec calls for a `docs/TESTING.md` containing the manual smoke checklist. Stage 4 introduces the file and seeds it with the Stage 4 entries. Future stages append.

- [ ] **Step 1: Check whether `docs/TESTING.md` exists**

```bash
ls /home/bhengen/dev/vibeflow/docs/TESTING.md
```

If it doesn't exist (expected for Stage 4), create it. If it does exist, this task only appends a "Stage 4" section to it.

- [ ] **Step 2: Create `docs/TESTING.md`**

Write the file with the following contents:

```markdown
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
- [ ] Stderr shows `[INFO] spawning first tab` with a `shell=` field reflecting
  your `$SHELL` (or `/bin/sh` if unset).
- [ ] Type `echo hi` then press Enter. Stderr shows passthrough log lines
  (visible at trace level: `RUST_LOG=vibeflow=trace`). You won't see the
  output in the window yet — that's Stage 5.
- [ ] Press Ctrl+C. No crash; stderr shows the byte going through.
- [ ] Resize the window by dragging the edge. No crash; no surface errors in
  stderr; the dark grey fills the new size cleanly.
- [ ] Resize down to a tiny window (~10 px on a side). No crash; no PTY errors.
- [ ] Press Ctrl+D at an empty prompt. Stderr shows `session died` and then
  `all tabs closed; exiting`. The window closes; the binary exits 0.
- [ ] Re-run; press the window-manager close button. Stderr shows
  `close requested; exiting`. Exit 0.
- [ ] Re-run on Wayland (if available) — the display server is selected
  automatically by winit. All checks above still pass.
- [ ] Re-run on X11 (set `WINIT_UNIX_BACKEND=x11`). All checks above still pass.

If the binary fails to start with a wgpu error, try `WGPU_BACKEND=gl`. If it
still fails, capture stderr and file a bug.
```

- [ ] **Step 3: Walk the checklist**

Run each item by hand against `target/debug/vibeflow`. Mark each item complete
in the file (replace `- [ ]` with `- [x]` for the items that pass; if any item
fails, note the failure mode and fix before tagging Stage 4).

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 4 manual smoke checklist (TESTING.md)"
```

---

## Task 10: Final verification + tag

**Files:** none (verification + git tag)

- [ ] **Step 1: Full local CI dry-run**

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

Expected: `ALL GREEN`. Test count should be: every Stage 3 test + the new ones from Stage 4 (Task 1: 2 lib tests; Task 6: 3 unit tests; Task 7: 8 unit tests = 13 new tests in `vibeflow` crate). The integration test count from Stage 3 is unchanged.

- [ ] **Step 2: 60-second fuzz on the protocol parser**

```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: clean.

- [ ] **Step 3: Re-run the manual smoke checklist**

Re-walk `docs/TESTING.md`'s Stage 4 section to confirm nothing regressed during the final fmt/clippy passes.

- [ ] **Step 4: Tag the milestone**

```bash
cd /home/bhengen/dev/vibeflow
git tag -a stage4-window-render-complete -m "winit window + wgpu clear-color render plumbing complete (Stage 4 of v0.1)"
git tag --list
```

- [ ] **Step 5: Surface to user**

Report:
- Number of new commits on this stage (should be 10).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 5 (alacritty_terminal grid + cell renderer) as the next plan.

---

## Spec coverage check

Mapping Stage 4 spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| Architecture — winit + wgpu single GUI process, main thread = event loop | Task 3 (WindowApp), Task 4 (EventLoop entry) |
| Components — `window.rs` (~180 LOC) | Tasks 3, 5, 6, 7, 8 |
| Components — `render/grid.rs` (placeholder, no grid yet) | Task 2 (Renderer skeleton; Stage 5 fills grid logic) |
| Process & threading model — main thread + reader threads via mpsc | Task 1 (master moved to main thread; reader thread unchanged) |
| Data flow A — Claude emits 'waiting' → state change observable | Task 5 (StateChanged events logged via tracing) |
| Data flow B — User keystroke → focused PTY child stdin | Task 7 (KeyboardInput → key_to_bytes → App::send_input) |
| Error handling — GPU init fatal with actionable message | Task 2 (Renderer::new wraps each failure with context) + Task 3 (resumed exits on Renderer error) |
| Error handling — Surface lost → reconfigure | Task 8 (SurfaceError::Lost / Outdated → reconfigure) |
| Error handling — Child exits → mark dead, freeze grid | Task 5 (Died → close_tab; banner UI is Stage 6) |
| Window resize → propagate to PTY | Task 6 (pixels_to_grid + WindowEvent::Resized → resize_all) |
| Logging via tracing crate, RUST_LOG=vibeflow=debug | Task 4 (tracing-subscriber init) |
| Default theme dark (#0e0e12 background) | Task 2 (CLEAR_COLOR constant) |

**Out of scope for Stage 4 (deferred):**
- Cell-grid rendering of terminal output — Stage 5 (alacritty_terminal + render/grid.rs).
- Tab bar with two-line tabs and Notice indicator — Stage 6.
- Font atlas + cosmic-text shaping — Stage 7.
- Full input handling (arrows, F-keys, Alt/Meta combos, mouse) — Stage 8.
- TOML config + hot-reload — Stage 9.
- Foreground-process detection driving `set_heuristic_active(true)` — Stage 9 (procfs polling on Linux).
- File logging + rotation under `~/.local/state/vibeflow/` — Stage 9.
- Shell hooks shipping (`shells/vibeflow.zsh` etc.) — Stage 10.
- Claude Code hooks integration — Stage 10.

## Self-review

- **Spec coverage:** every Stage 4-relevant spec requirement maps to a task. Stage 5+ items are explicitly listed as out of scope.
- **Placeholder scan:** no `TBD`/`TODO`/`implement later`/`similar to` patterns. Each step has actual code or actual commands.
- **Type consistency check:**
  - `WindowApp { window: Option<Arc<Window>>, renderer: Option<Renderer>, app: App }` — used identically across Tasks 3, 5, 6, 7, 8.
  - `Renderer { _window: Arc<Window>, surface, device, queue, surface_config }` (Task 2) — `surface_size` (used in Task 6's resize math), `resize` (Task 6 callsite), `render` (Tasks 3 + 8 callsites), `reconfigure` (Task 8 callsite) all match the type definition.
  - `pixels_to_grid(width_px: u32, height_px: u32, cell_w: u32, cell_h: u32) -> (u16, u16)` — used identically in Task 6's tests and the WindowEvent::Resized handler.
  - `key_to_bytes(event: &KeyEvent) -> Option<Vec<u8>>` — used identically in Task 7's tests and the KeyboardInput handler.
  - `PtySession::resize(&self, rows: u16, cols: u16) -> io::Result<()>` and `App::resize_all(&self, rows: u16, cols: u16) -> io::Result<()>` agree on signature; both take `&self`.
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** `master` moves to the main thread (`PtySession` field) per Task 1. The reader thread no longer captures it. The mpsc channel is still the only cross-thread communication path. `Renderer` stays on the main thread (its wgpu types are not `Send`). Matches the spec's threading model exactly.
- **Forward-declared item handling:** `Renderer` may need temporary `#[allow(dead_code)]` between Tasks 2 and 3; the annotation is removed in Task 3 once `WindowApp` constructs one. No suppressions linger past the task that introduces their first user.
- **GUI testability:** Tasks 1, 6, 7 use TDD for pure logic. Tasks 2, 3, 4, 5, 8 are inherently imperative and verified by `cargo build` + the manual smoke checklist (Task 9). The plan is honest about this trade-off in the "A note on testing this stage" preamble.
- **Pedagogical clarity (user is learning Rust):** the plan includes explicit "Why" explanations for non-obvious choices: `Arc<Window>` for surface borrow lifetime (Task 2), `pollster::block_on` for sync-in-practice async wgpu init (Task 2), `as_mut()` vs `as_ref()` for surface reconfigure (Task 8), `&self` vs `&mut self` for resize (Task 1), the lazy `resumed` lifecycle (Task 3), and the 100ms fixed wake-up vs computed deadline trade-off (Task 5). Preserve verbatim during execution.
