# vibeflow Stage 3 Implementation Plan: PtySession + reader threads + headless App

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire a real PTY child process up to the dispatcher and tracker built in Stage 2. After this plan, vibeflow is a binary crate that spawns a shell, reads its output through the OSC dispatcher into the tracker, and surfaces tab-state changes — all without a window or rendering. The "headless terminal that knows AI state" demo.

**Architecture:** Three new modules in the `vibeflow` crate, glued together by a `[[bin]]` target:

- `session/pty.rs` — thin wrapper around `portable-pty`. Spawns a child shell on a pseudoterminal, returns the master read/write halves and a `Child` handle.
- `session/mod.rs` (extended) — `PtySession` owns one tab's PTY child, a reader thread that blocks on `read()` and sends bytes via `mpsc::Sender<Vec<u8>>` to the main thread, plus the per-session `OscDispatcher`, `AiStateTracker`, and the byte-receiver. `poll()` drains the channel and runs bytes through dispatcher → tracker.
- `app.rs` — single-threaded `App` owning `Vec<PtySession>`, active tab index, default `TrackerConfig`. `poll_all()` drives every session; `tick(now)` calls every session's `AiStateTracker::tick`.
- `bin/main.rs` — minimal "headless demo" binary. Spawns a single shell, runs for a few seconds, prints any tab-state changes to stdout. Stage 4 replaces this with the winit event loop.

The threading model from the spec stays exact: main thread mutates all state and never blocks on PTY reads; reader threads block on `read()` and send bytes via mpsc. No tokio. No `Arc<Mutex<…>>` on the tracker (per spec — main thread owns it).

**Tech Stack:** Adds `portable-pty = "0.8"` (cross-platform-clean PTY abstraction). No other new external deps. The Stage 1/2 protocol crate, dispatcher, and tracker are reused as-is.

**Stage scope:** This plan covers Stage 3 only. Stage 4 introduces winit/wgpu and the actual window. Stage 3 ends with a runnable binary that demonstrates the data flow PTY → bytes → dispatcher → events → tracker → state — verified by integration tests that spawn fake-AI child processes via `/bin/sh -c "..."` and assert observed state transitions.

**Lessons carried forward from Stages 1–2:**
- Pre-fmt the verbatim Rust code (rustfmt prefers wider line breaking than the human-readable plan style).
- Forward-declared items get `#[allow(dead_code)]` until the first lib-level caller arrives, with cleanup in the introducing-caller task.
- Time injection (`now: Instant` as parameter) for everything the tracker touches — the main thread always has `Instant::now()` in hand.
- Per-task `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings` verify step before commit.
- For tests that depend on subprocess behaviour (this stage's integration tests), use `/bin/sh -c "..."` invocations directly rather than depending on a particular shell binary at a particular path.

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Add `portable-pty = "0.8"` dep, add `[[bin]] name = "vibeflow"` target. |
| `crates/vibeflow/src/session/pty.rs` (new) | `spawn_pty(cmd) -> Result<PtyHandles>` wrapper around `portable_pty`. ~80 LOC. |
| `crates/vibeflow/src/session/mod.rs` (modify) | Add `pub mod pty;` declaration; re-export `PtySession`. |
| `crates/vibeflow/src/session/session.rs` (new) | `PtySession` struct + reader-thread plumbing + `poll`, `send_input`, `tick`, `is_alive`. ~180 LOC. |
| `crates/vibeflow/src/app.rs` (new) | `App` struct + `Vec<PtySession>` orchestration + `new_tab`, `close_tab`, `poll_all`, `tick_all`. ~140 LOC. |
| `crates/vibeflow/src/lib.rs` (modify) | Add `pub mod app;` declaration. |
| `crates/vibeflow/src/main.rs` (new) | Tiny driver binary — spawns one tab, polls for ~5 s, prints state changes. ~50 LOC. |
| `crates/vibeflow/tests/pty_integration.rs` (new, Task 11) | End-to-end: fake-AI child via `/bin/sh -c "..."` → byte flow → tracker state → assert. |

---

## Task 0: Add `portable-pty` dep + `[[bin]]` target + module declarations

**Files:**
- Modify: `crates/vibeflow/Cargo.toml`
- Modify: `crates/vibeflow/src/lib.rs`
- Modify: `crates/vibeflow/src/session/mod.rs`
- Create: stubs for `crates/vibeflow/src/session/pty.rs`, `crates/vibeflow/src/session/session.rs`, `crates/vibeflow/src/app.rs`, `crates/vibeflow/src/main.rs`

- [ ] **Step 1: Add `portable-pty` dep and `[[bin]]` target**

Edit `crates/vibeflow/Cargo.toml`. Replace the existing `[dependencies]` and `[lib]` sections with:

```toml
[lib]
path = "src/lib.rs"

[[bin]]
name = "vibeflow"
path = "src/main.rs"

[dependencies]
vibeflow-protocol = { path = "../vibeflow-protocol", version = "0.1" }
portable-pty = "0.8"
```

(Leave the rest of the manifest — `[package]`, `[lints]`, `[dev-dependencies]` — unchanged.)

**Why a `[[bin]]` target now:** Stage 3 needs a runnable program for the integration tests and the headless demo. The crate becomes both a library (the modules other stages will import) and a binary (the user-facing terminal). Cargo handles both targets cleanly.

- [ ] **Step 2: Declare the new modules**

Edit `crates/vibeflow/src/lib.rs`. Replace the contents with:

```rust
//! `vibeflow` — GPU-accelerated terminal emulator for Linux that signals AI-tool state.
//!
//! Stage 3 of v0.1 wires up a real PTY child process behind the streaming OSC
//! dispatcher and the per-tab state tracker introduced in Stages 1–2. Stage 4
//! introduces the window and rendering. Public surface: [`session`] and [`app`].
//!
//! See `docs/superpowers/specs/2026-05-01-vibeflow-design.md` for the full design.

pub mod app;
pub mod session;
```

Edit `crates/vibeflow/src/session/mod.rs`. Replace the contents with:

```rust
//! Per-tab session machinery: PTY plumbing, OSC dispatching, AI-state tracking.

pub mod osc;
pub mod pty;
pub mod session;
pub mod tracker;

pub use session::{PtySession, SessionEvent};
```

- [ ] **Step 3: Stub the new files**

Write `crates/vibeflow/src/session/pty.rs`:

```rust
//! Thin wrapper around `portable-pty`. Exposes [`spawn_pty`] which returns a
//! [`PtyHandles`] containing the bits the caller needs to drive a child process
//! on a pseudoterminal: the master reader, the master writer, and the child
//! process handle for liveness checks and explicit kill.
```

Write `crates/vibeflow/src/session/session.rs`:

```rust
//! `PtySession` — one tab's PTY child, reader thread, OSC dispatcher, and
//! AI-state tracker, all driven from the main thread via a single-producer
//! single-consumer channel.
```

Write `crates/vibeflow/src/app.rs`:

```rust
//! `App` — single-threaded "central authority" that owns every tab's
//! [`PtySession`] and orchestrates polling and timeout ticks.
```

Write `crates/vibeflow/src/main.rs`:

```rust
//! Headless demo binary: spawn one shell, poll for state changes, print them.
//!
//! Stage 4 replaces this with the winit event loop and the wgpu renderer.
//! For Stage 3 it just exercises the PTY → dispatcher → tracker pipeline so
//! integration tests have something to compile against.

fn main() {
    eprintln!("vibeflow Stage 3: headless demo not yet implemented");
}
```

- [ ] **Step 4: Verify the workspace builds**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build (the new modules are stubs but valid). Clippy silent.

If clippy complains about `unused_imports` for `pub use session::{PtySession, SessionEvent};` (those types don't exist yet), that's a real problem — the line should still compile because it's a `pub use` of items in a sibling module that hasn't defined them. **If the line fails to compile, comment it out for now and re-introduce in Task 2.** This is a "use of undeclared crate or module" error. The plan creates the items in Task 2; if the compiler is strict about the pub use ahead-of-time, defer it.

Pragmatic workaround if Step 4 fails: change the line in `mod.rs` to:

```rust
pub use session::PtySession;  // SessionEvent re-export added in Task 2
```

…and the test still validates compilation. If even this fails (because `session::PtySession` doesn't exist), comment the whole `pub use` line and add it in Task 2's verbatim code.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add Cargo.toml crates/vibeflow/Cargo.toml crates/vibeflow/src/lib.rs crates/vibeflow/src/session/mod.rs crates/vibeflow/src/session/pty.rs crates/vibeflow/src/session/session.rs crates/vibeflow/src/app.rs crates/vibeflow/src/main.rs Cargo.lock
git commit -m "chore(vibeflow): add portable-pty dep, [[bin]] target, module stubs"
```

(`Cargo.lock` will have grown — `portable-pty` and its transitive deps are added.)

---

## Task 1: `pty::spawn_pty` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/pty.rs`

The PTY wrapper is small and platform-specific. We test it against `/bin/sh` since that's available on every Linux+CI box. The contract:

- `spawn_pty(argv: &[&str]) -> std::io::Result<PtyHandles>`
- `PtyHandles { reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>, child: Box<dyn Child + Send + Sync> }`
- `child.try_wait()` checks liveness without blocking; `child.kill()` terminates.

- [ ] **Step 1: Write the failing test**

Replace the contents of `crates/vibeflow/src/session/pty.rs` with:

```rust
//! Thin wrapper around `portable-pty`. Exposes [`spawn_pty`] which returns a
//! [`PtyHandles`] containing the bits the caller needs to drive a child process
//! on a pseudoterminal: the master reader, the master writer, and the child
//! process handle for liveness checks and explicit kill.

use std::io::{Read, Write};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};

/// Handles returned from [`spawn_pty`]. The fields are owned by separate
/// threads in `PtySession`: the reader is moved into the reader thread,
/// the writer stays on the main thread, the child is owned by `PtySession`
/// for liveness checks and explicit kill, and the master must be kept alive
/// alongside the reader (its drop closes the PTY).
pub struct PtyHandles {
    /// Read half of the PTY master. Move to a reader thread.
    pub reader: Box<dyn Read + Send>,
    /// Write half of the PTY master. Used by the main thread for keyboard input.
    pub writer: Box<dyn Write + Send>,
    /// The child process. Drop or kill to terminate.
    pub child: Box<dyn Child + Send + Sync>,
    /// The master PTY. Keep alive as long as `reader` is in use — once the
    /// box is dropped, the PTY closes and reads return EOF. Callers should
    /// move it into the same scope as the reader (typically the reader thread).
    pub master: Box<dyn MasterPty + Send>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::time::Duration;

    #[test]
    fn spawn_sh_echo_reads_back_the_string() {
        // Spawn `sh -c "printf hello"`. The child writes "hello" to stdout
        // (which is the PTY slave), then exits. We read from the master.
        let handles = spawn_pty(&["/bin/sh", "-c", "printf hello"]).unwrap();
        let mut reader = handles.reader;
        let mut buf = Vec::new();

        // Read until EOF or until we have at least 5 bytes. Terminals translate
        // \n to \r\n on output by default, so we only check for the literal
        // bytes "hello" — printf without a newline avoids that translation.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut chunk = [0u8; 64];
        loop {
            if std::time::Instant::now() >= deadline {
                panic!("timed out waiting for `hello`; got: {:?}", buf);
            }
            match reader.read(&mut chunk) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() >= 5 {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        assert!(buf.starts_with(b"hello"), "expected `hello` prefix, got {:?}", buf);
    }
}
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::pty
```

Expected: compile error — `spawn_pty` not found.

- [ ] **Step 2: Implement `spawn_pty`**

Append to `crates/vibeflow/src/session/pty.rs` (above the `#[cfg(test)] mod tests` block):

```rust
/// Spawn a child process on a pseudoterminal.
///
/// `argv` is the command + arguments — `argv[0]` is the program path. PTY size
/// defaults to 80x24; resizing is added in Stage 6 (window event handler).
///
/// # Errors
/// Returns an `io::Error` if the PTY cannot be opened or the child cannot be
/// spawned. Wraps `portable_pty`'s typed errors via `io::Error::other`.
pub fn spawn_pty(argv: &[&str]) -> std::io::Result<PtyHandles> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(std::io::Error::other)?;

    let mut cmd = CommandBuilder::new(argv[0]);
    for arg in &argv[1..] {
        cmd.arg(arg);
    }
    // Set TERM so children behave reasonably. `xterm-256color` is a safe
    // baseline; Stage 6 may switch to `vibeflow` once we register a terminfo.
    cmd.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(std::io::Error::other)?;
    // Drop the slave so the master is the only end of the PTY — reads will
    // see EOF only when the child exits.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(std::io::Error::other)?;
    let writer = pair
        .master
        .take_writer()
        .map_err(std::io::Error::other)?;

    Ok(PtyHandles {
        reader,
        writer,
        child,
        master: pair.master,
    })
}
```

- [ ] **Step 3: Run the test**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::pty
```

Expected: `test result: ok. 1 passed; 0 failed`. The test spawns a real `/bin/sh -c "printf hello"`, reads `hello` from the master, and asserts.

If the test hangs or times out: PTY availability on the test environment may be the issue. CI runs on `ubuntu-latest` which has full PTY support. Local Linux is fine. WSL/Docker without `/dev/ptmx` is not. **If the test fails on the local box but compiles**, run it manually: `cargo test -p vibeflow --lib session::pty -- --nocapture` and inspect the buffer contents.

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/pty.rs
git commit -m "feat(session): pty::spawn_pty wrapper around portable-pty"
```

---

## Task 2: `PtySession` skeleton + reader thread + bytes channel (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`
- Modify: `crates/vibeflow/src/session/mod.rs` (re-add `pub use` if Task 0 deferred it)

`PtySession` owns the PTY child, the writer half, the reader-thread `JoinHandle`, the bytes-channel `Receiver`, the per-session `OscDispatcher`, and the `AiStateTracker`. The reader thread loops on `reader.read()` and sends `Vec<u8>` chunks via `mpsc` to the main thread.

This task introduces the type, the constructor (which spawns the reader thread), and a public accessor `state()` that just delegates to the tracker. `poll`/`send_input`/`tick` arrive in Tasks 3–5.

`SessionEvent` is the public event type the App consumes per session — it wraps `DispatchEvent` plus a "session died" signal.

- [ ] **Step 1: Write the failing test**

Replace the contents of `crates/vibeflow/src/session/session.rs` with:

```rust
//! `PtySession` — one tab's PTY child, reader thread, OSC dispatcher, and
//! AI-state tracker, all driven from the main thread via a single-producer
//! single-consumer channel.

use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use portable_pty::Child;

use crate::session::osc::{DispatchEvent, OscDispatcher};
use crate::session::pty::{spawn_pty, PtyHandles};
use crate::session::tracker::{AiStateTracker, TabState, TrackerConfig, TrackerInput};

/// Public event type the `App` observes from a session, beyond just the
/// underlying [`DispatchEvent`]. `Died` lets the App detect when the child
/// exits and the reader thread has finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// State of the per-session tracker just changed to this value.
    StateChanged(TabState),
    /// Bytes that should be forwarded to the terminal grid (Stage 4+ wires
    /// alacritty_terminal to consume these).
    PassThrough(Vec<u8>),
    /// The child exited or the reader thread terminated. After this event,
    /// `is_alive()` returns false and further `poll()` calls produce nothing.
    Died,
}

/// One terminal tab's per-session machinery.
pub struct PtySession {
    /// Drains here when the reader thread sends bytes from the PTY master.
    rx: Receiver<Vec<u8>>,
    /// Used by [`send_input`] to write keystrokes to the PTY master.
    writer: Box<dyn Write + Send>,
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

impl PtySession {
    /// Spawn a child via the given `argv` on a fresh pseudoterminal and start
    /// the reader thread. The reader thread runs until the PTY hits EOF
    /// (typically on child exit), then the channel disconnects.
    ///
    /// # Errors
    /// Propagates PTY-spawn or thread-creation failures.
    pub fn spawn(argv: &[&str], config: TrackerConfig) -> std::io::Result<Self> {
        let PtyHandles {
            reader,
            writer,
            child,
            master,
        } = spawn_pty(argv)?;
        // The reader-thread closure owns the master so its drop coincides
        // with the reader thread's exit (when the child closes the PTY).
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut reader = reader;
        let reader_thread = thread::Builder::new()
            .name("vibeflow-pty-reader".into())
            .spawn(move || {
                let _master_alive = master; // keep alive for the read loop
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                // Receiver was dropped — session is closing.
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
            child,
            reader_thread: Some(reader_thread),
            dispatcher: OscDispatcher::new(),
            tracker: AiStateTracker::new(config),
            alive: true,
        })
    }

    /// Current visual state of this session's tab.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.tracker.state()
    }

    /// Whether the child is still running and the reader thread alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Best-effort kill, then join the reader thread so it doesn't outlive
        // the session. Errors are swallowed because we're tearing down anyway.
        let _ = self.child.kill();
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn session_spawns_and_reports_state() {
        // `sleep 5` exits cleanly; the session is alive immediately after spawn
        // and reports the default Active state.
        let s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
        assert!(s.is_alive());
        assert_eq!(s.state(), TabState::Active);
        // Drop `s` here — the Drop impl kills the child and joins the reader.
        drop(s);
    }

    #[test]
    fn session_reader_thread_pumps_bytes_to_channel() {
        // Spawn a child that prints predictable bytes, then read from the
        // session's channel directly to verify the reader thread is alive.
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "printf hello"], TrackerConfig::default())
                .unwrap();
        // Drain the channel for up to 2s and accumulate bytes.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buf = Vec::new();
        while Instant::now() < deadline && buf.len() < 5 {
            match s.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => buf.extend_from_slice(&chunk),
                Err(_) => continue,
            }
        }
        assert!(buf.starts_with(b"hello"), "got: {:?}", buf);
    }
}
```

If you commented out `pub use session::{PtySession, SessionEvent};` in Task 0 (because the items didn't exist yet), restore it now in `crates/vibeflow/src/session/mod.rs`. The line should read:

```rust
pub use session::{PtySession, SessionEvent};
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: 2 new tests pass (the test code references `PtySession`, `TrackerConfig`, etc., which exist now).

If a test hangs on the channel-recv loop: the reader thread may have died early. Inspect by running `cargo test -p vibeflow --lib session::session -- --nocapture`.

- [ ] **Step 2: Verify fmt + clippy + full test suite**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
cargo test -p vibeflow
```

Expected: silent for the first two; full test suite reports all prior tests + the 2 new ones.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/session.rs crates/vibeflow/src/session/mod.rs
git commit -m "feat(session): PtySession with reader thread and bytes channel"
```

---

## Task 3: `PtySession::poll` — drain channel into dispatcher → tracker → events (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`

`poll(now)` is the heart of the session. It pulls every pending byte chunk off the channel, runs each chunk through the dispatcher, and routes each `DispatchEvent` to the tracker plus emits a `SessionEvent` for the App. State changes produce `SessionEvent::StateChanged`. Pass-through bytes also count as observed output (the tracker's heuristic-silence baseline).

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block in `crates/vibeflow/src/session/session.rs`:

```rust
    use vibeflow_protocol::{Frame as ProtoFrame, State as ProtoState};

    #[test]
    fn poll_routes_osc_1338_through_dispatcher_and_tracker() {
        // Spawn a child that prints exactly one OSC 1338 sequence, then exits.
        // The session's poll() should observe a state change to Working.
        let bytes = ProtoFrame::new(ProtoState::Working).to_bytes();
        let bytes_str = bytes
            .iter()
            .map(|b| format!("\\x{b:02x}"))
            .collect::<String>();
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", &format!("printf '{bytes_str}'")],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        let mut state_changed_to_working = false;
        while Instant::now() < deadline && !state_changed_to_working {
            for ev in s.poll(Instant::now()) {
                if matches!(ev, SessionEvent::StateChanged(TabState::Working)) {
                    state_changed_to_working = true;
                }
                events.push(ev);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            state_changed_to_working,
            "expected StateChanged(Working); got events: {events:?}"
        );
        assert_eq!(s.state(), TabState::Working);
    }
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: compile error — `poll` not defined.

- [ ] **Step 2: Implement `poll`**

Add the following method to `impl PtySession`, between `state` and `is_alive`:

```rust
    /// Drain every pending byte chunk off the reader channel, run each through
    /// the dispatcher, route resulting events into the tracker, and return the
    /// public-facing [`SessionEvent`]s for the App. Non-blocking — returns
    /// immediately if the channel is empty.
    pub fn poll(&mut self, now: Instant) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => {
                    for ev in self.dispatcher.feed(&chunk) {
                        match ev {
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
                            DispatchEvent::PassThrough(bytes) => {
                                self.tracker.on_input(TrackerInput::OutputObserved, now);
                                events.push(SessionEvent::PassThrough(bytes));
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Reader thread is gone — child probably exited.
                    if self.alive {
                        self.alive = false;
                        events.push(SessionEvent::Died);
                    }
                    break;
                }
            }
        }
        events
    }
```

- [ ] **Step 3: Run tests**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: 3 new tests pass (the previous 2 from Task 2 + 1 new). Total in the session module: 3.

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/session.rs
git commit -m "feat(session): PtySession::poll routes bytes through dispatcher and tracker"
```

---

## Task 4: `PtySession::send_input` — write keystrokes to PTY master (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`

`send_input` is what the keyboard handler in Stage 5+ will call. For Stage 3, we just verify that bytes written to the master end show up on the slave end (i.e., the child sees them) by spawning `cat` and verifying the round-trip.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block:

```rust
    #[test]
    fn send_input_round_trips_bytes_through_pty() {
        // Spawn `cat`, send some bytes to its stdin via send_input, verify
        // the same bytes come back through the reader channel (since cat
        // echoes its input to stdout). Send EOT (0x04) to make cat exit.
        let mut s = PtySession::spawn(&["/bin/cat"], TrackerConfig::default()).unwrap();
        s.send_input(b"hello\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = Vec::new();
        while Instant::now() < deadline && !buf.windows(5).any(|w| w == b"hello") {
            match s.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => buf.extend_from_slice(&chunk),
                Err(_) => continue,
            }
        }
        assert!(
            buf.windows(5).any(|w| w == b"hello"),
            "expected `hello` in echoed buffer; got: {buf:?}"
        );
        // Tell cat to exit.
        s.send_input(&[0x04]).unwrap();
    }
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: compile error — `send_input` not defined.

- [ ] **Step 2: Implement `send_input`**

Add to `impl PtySession`, after `poll`:

```rust
    /// Write keystroke bytes to the PTY master. The child sees these as input
    /// on its stdin.
    ///
    /// # Errors
    /// Propagates any underlying `io::Error` from the writer.
    pub fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }
```

- [ ] **Step 3: Run tests**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: 4 new tests pass (3 prior + 1 new).

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/session.rs
git commit -m "feat(session): PtySession::send_input writes keystrokes to PTY master"
```

---

## Task 5: `PtySession::tick` — surface tracker timeouts (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`

The tracker's `tick(now)` fires the heuristic-silence and stale-state timeouts. The session needs to expose this so the App can call it on a timer (Stage 4+ uses winit's `ControlFlow::WaitUntil`; Stage 3's main.rs uses a simple sleep loop). The session's `tick` returns a `SessionEvent::StateChanged` when a timeout fires.

Also adds `set_heuristic_active(bool)` so the App can flip Tier 3 on for known AI processes (Stage 6 wires this up to a foreground-process check; for Stage 3 the test sets it manually).

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn tick_does_not_fire_within_timeout_windows() {
        let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default())
            .unwrap();
        // Default config: stale_state 30s, heuristic_silence 4s — neither
        // fires within 1s of spawn.
        let evs = s.tick(Instant::now() + Duration::from_secs(1));
        assert_eq!(evs, vec![]);
    }

    #[test]
    fn tick_fires_stale_state_timeout() {
        let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 60"], TrackerConfig::default())
            .unwrap();
        let now = Instant::now();
        // Simulate state change by feeding an AiFrame manually to set
        // last_event_at, then tick past the 30 s stale-state window.
        let frame_bytes =
            vibeflow_protocol::Frame::new(vibeflow_protocol::State::Working).to_bytes();
        // Feed bytes directly (not via the PTY) to control timing.
        for ev in s.dispatcher.feed(&frame_bytes) {
            if let DispatchEvent::AiState(frame) = ev {
                s.tracker.on_input(TrackerInput::AiFrame(frame), now);
            }
        }
        assert_eq!(s.state(), TabState::Working);

        let evs = s.tick(now + Duration::from_secs(31));
        assert_eq!(evs, vec![SessionEvent::StateChanged(TabState::Active)]);
        assert_eq!(s.state(), TabState::Active);
    }

    #[test]
    fn set_heuristic_active_toggles_tier_3() {
        // Direct test that the toggle reaches the tracker.
        let mut s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default())
            .unwrap();
        s.set_heuristic_active(true);
        // No assertion on internal tracker state (it's a private field) —
        // exercise the path via tick after a Working transition + observed
        // output to ensure heuristic fires when the flag is on.
        let now = Instant::now();
        let frame_bytes =
            vibeflow_protocol::Frame::new(vibeflow_protocol::State::Working).to_bytes();
        for ev in s.dispatcher.feed(&frame_bytes) {
            if let DispatchEvent::AiState(frame) = ev {
                s.tracker.on_input(TrackerInput::AiFrame(frame), now);
            }
        }
        s.tracker.on_input(TrackerInput::OutputObserved, now);

        let evs = s.tick(now + Duration::from_secs(5));
        assert_eq!(evs, vec![SessionEvent::StateChanged(TabState::Waiting)]);
    }
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: compile errors — `tick` and `set_heuristic_active` not defined on `PtySession`. The third test also accesses `s.dispatcher` and `s.tracker` directly — these are *private fields*, only callable from inside the `mod tests`. Tests in the same module CAN access private fields; this is a Rust idiom. The `#[cfg(test)] mod tests` block sees through the privacy barrier.

- [ ] **Step 2: Implement `tick` and `set_heuristic_active`**

Add to `impl PtySession`, after `send_input`:

```rust
    /// Run the tracker's timeout checks at `now`. Returns a [`SessionEvent`]
    /// per timeout-driven state change (currently zero or one event).
    pub fn tick(&mut self, now: Instant) -> Vec<SessionEvent> {
        if self.tracker.tick(now) {
            vec![SessionEvent::StateChanged(self.tracker.state())]
        } else {
            Vec::new()
        }
    }

    /// Toggle the Tier 3 heuristic-silence inference. The App calls this when
    /// the foreground process matches the configured AI-tool list.
    pub fn set_heuristic_active(&mut self, active: bool) {
        self.tracker.set_heuristic_active(active);
    }
```

- [ ] **Step 3: Run tests**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: 7 tests pass total in this module (4 prior + 3 new).

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/session/session.rs
git commit -m "feat(session): PtySession::tick surfaces tracker timeout transitions"
```

---

## Task 6: `App` skeleton + `new_tab` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/app.rs`

`App` owns `Vec<PtySession>` and tracks the active tab index. `new_tab(argv)` spawns a session and pushes it onto the vec. `close_tab(idx)` removes a session (its `Drop` kills the child). `tabs()` returns a slice of references for inspection.

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/vibeflow/src/app.rs` with:

```rust
//! `App` — single-threaded "central authority" that owns every tab's
//! [`PtySession`] and orchestrates polling and timeout ticks.

use crate::session::{PtySession, SessionEvent};
use crate::session::tracker::TrackerConfig;

/// Default per-tracker config used for every new tab. Stage 8 will replace
/// this with a TOML-loaded config sourced from `~/.config/vibeflow/config.toml`.
fn default_tracker_config() -> TrackerConfig {
    TrackerConfig::default()
}

/// Single-threaded central authority for the terminal app: owns every tab,
/// dispatches polls and ticks across them, tracks the focused tab.
pub struct App {
    tabs: Vec<PtySession>,
    active: usize,
    tracker_config: TrackerConfig,
}

impl App {
    /// Create an empty `App` with no tabs. Call [`new_tab`] to spawn the first.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            tracker_config: default_tracker_config(),
        }
    }

    /// Spawn a new tab. Returns the index of the new tab in [`tabs`]. The new
    /// tab becomes the active tab.
    ///
    /// # Errors
    /// Propagates any failure from [`PtySession::spawn`].
    pub fn new_tab(&mut self, argv: &[&str]) -> std::io::Result<usize> {
        let session = PtySession::spawn(argv, self.tracker_config)?;
        self.tabs.push(session);
        let idx = self.tabs.len() - 1;
        self.active = idx;
        Ok(idx)
    }

    /// Close (and drop) the tab at `idx`. The session's `Drop` kills the child
    /// and joins the reader thread.
    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let _dropped = self.tabs.remove(idx);
        if self.active >= self.tabs.len() && !self.tabs.is_empty() {
            self.active = self.tabs.len() - 1;
        }
    }

    /// Snapshot of all sessions (for read-only inspection — Stage 4+ tab-bar
    /// renderer uses this to draw indicator stripes).
    #[must_use]
    pub fn tabs(&self) -> &[PtySession] {
        &self.tabs
    }

    /// Index of the currently focused tab. Valid only when `tabs()` is non-empty.
    #[must_use]
    pub fn active(&self) -> usize {
        self.active
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tracker::TabState;

    #[test]
    fn new_app_has_no_tabs() {
        let app = App::new();
        assert!(app.tabs().is_empty());
    }

    #[test]
    fn new_tab_spawns_and_focuses() {
        let mut app = App::new();
        let idx = app
            .new_tab(&["/bin/sh", "-c", "sleep 5"])
            .unwrap();
        assert_eq!(idx, 0);
        assert_eq!(app.tabs().len(), 1);
        assert_eq!(app.active(), 0);
        assert_eq!(app.tabs()[0].state(), TabState::Active);
    }

    #[test]
    fn close_tab_removes_session() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        assert_eq!(app.tabs().len(), 2);
        app.close_tab(0);
        assert_eq!(app.tabs().len(), 1);
    }

    #[test]
    fn close_tab_with_invalid_index_is_a_no_op() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.close_tab(99); // out of range
        assert_eq!(app.tabs().len(), 1);
    }

    #[test]
    fn _unused_session_event_silences_dead_code() {
        // Force a use of SessionEvent so its `Died` variant isn't reported as
        // unread until App::poll_all (Task 8) wires it through.
        let _ = SessionEvent::Died;
    }
}
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
```

Expected: 5 new tests pass (and 1 of them — `_unused_session_event_silences_dead_code` — is a no-op test there only to silence dead_code warnings on the `SessionEvent::Died` variant before Task 8 wires it through).

- [ ] **Step 2: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent. If clippy flags `SessionEvent::Died` as unused, the test in step 1 should suppress it. If it doesn't, fall back to a `#[allow(dead_code)]` on the variant in `session.rs`.

- [ ] **Step 3: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/app.rs
git commit -m "feat(app): App skeleton with new_tab and close_tab"
```

---

## Task 7: `App::poll_all` — drain every session into one event stream (TDD)

**Files:**
- Modify: `crates/vibeflow/src/app.rs`

`poll_all(now)` walks every session, calls its `poll(now)`, and returns a `Vec<(usize, SessionEvent)>` — pairs of (tab index, event). The App's caller (the main loop in Stage 4+, or the integration test in Task 11) iterates and reacts: `Died` → close the tab, `StateChanged` → repaint that tab's indicator, etc.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block:

```rust
    use std::time::{Duration, Instant};
    use vibeflow_protocol::{Frame as ProtoFrame, State as ProtoState};

    #[test]
    fn poll_all_collects_state_changes_from_each_session() {
        let mut app = App::new();
        // Tab 0: emits a single OSC 1338 working frame, then exits.
        let bytes = ProtoFrame::new(ProtoState::Working).to_bytes();
        let bytes_str = bytes
            .iter()
            .map(|b| format!("\\x{b:02x}"))
            .collect::<String>();
        app.new_tab(&[
            "/bin/sh",
            "-c",
            &format!("printf '{bytes_str}'"),
        ])
        .unwrap();

        // Poll for up to 5s, looking for a StateChanged(Working) event from
        // tab 0.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while Instant::now() < deadline && !found {
            for (idx, ev) in app.poll_all(Instant::now()) {
                use crate::session::tracker::TabState;
                if idx == 0
                    && matches!(ev, SessionEvent::StateChanged(TabState::Working))
                {
                    found = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(found, "expected tab 0 to transition to Working");
    }
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
```

Expected: compile error — `poll_all` not defined.

- [ ] **Step 2: Implement `poll_all`**

Add to `impl App`, after `tabs`:

```rust
    /// Drive every session's [`PtySession::poll`] at `now` and collect the
    /// resulting events with their tab index. Returned vector is in
    /// `(tab_index, event)` pairs ordered by tab; the caller can iterate and
    /// react.
    pub fn poll_all(
        &mut self,
        now: std::time::Instant,
    ) -> Vec<(usize, SessionEvent)> {
        let mut all = Vec::new();
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            for ev in tab.poll(now) {
                all.push((idx, ev));
            }
        }
        all
    }
```

- [ ] **Step 3: Run tests**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
```

Expected: 6 tests pass (5 prior + 1 new).

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/app.rs
git commit -m "feat(app): App::poll_all collects per-tab events"
```

---

## Task 8: `App::tick_all` — fan out timeout ticks to every session (TDD)

**Files:**
- Modify: `crates/vibeflow/src/app.rs`

`tick_all(now)` calls `tick(now)` on every session and collects any state-change events. The App's main loop pairs `poll_all` (every iteration) with `tick_all` (also every iteration; tracker timeouts are idempotent if no window has elapsed).

This task also removes the `_unused_session_event_silences_dead_code` test from Task 6 — the dead_code is fixed by this task naturally because `poll_all` now returns `SessionEvent`s including `Died`. Confirm by running clippy after the test removal.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block:

```rust
    #[test]
    fn tick_all_returns_empty_when_no_timeouts_have_fired() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        let evs = app.tick_all(Instant::now() + Duration::from_secs(1));
        assert!(evs.is_empty());
    }
```

(The complementary "tick fires stale-state across the App" test is part of the integration test in Task 11 — easier to write there with full control over time.)

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
```

Expected: compile error — `tick_all` not defined.

- [ ] **Step 2: Implement `tick_all` and remove the dead-code-silencing test**

Add to `impl App`, after `poll_all`:

```rust
    /// Run [`PtySession::tick`] on every session at `now` and collect any
    /// timeout-driven [`SessionEvent`]s with their tab index.
    pub fn tick_all(
        &mut self,
        now: std::time::Instant,
    ) -> Vec<(usize, SessionEvent)> {
        let mut all = Vec::new();
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            for ev in tab.tick(now) {
                all.push((idx, ev));
            }
        }
        all
    }
```

Then **delete the test** `_unused_session_event_silences_dead_code` from Task 6 — `SessionEvent::Died` is now reachable via `tick_all` (and `poll_all` from Task 7), so the manual silencing isn't needed.

- [ ] **Step 3: Run tests**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
```

Expected: 6 tests pass (the prior 5 minus the deleted test plus the 2 new — net +1, total 6 in app module).

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

If clippy still complains about `SessionEvent::Died` being unused (because the test was the only thing referencing it), add `#[allow(dead_code)]` to the variant in `session.rs` with a comment "first lib-level user is the App's main loop in Stage 4". This is justified — Stage 3's main.rs and tests won't naturally observe the variant unless the test child segfaults.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/app.rs crates/vibeflow/src/session/session.rs
git commit -m "feat(app): App::tick_all fans out tracker timeouts"
```

---

## Task 9: `App::send_input` — write keystrokes to the active tab (TDD)

**Files:**
- Modify: `crates/vibeflow/src/app.rs`

`send_input(bytes)` writes to whichever tab is currently active. Stage 5+ keyboard-handler will call this with each KeyboardInput event. For Stage 3 we just verify the round-trip.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block:

```rust
    #[test]
    fn send_input_writes_to_active_tab() {
        let mut app = App::new();
        app.new_tab(&["/bin/cat"]).unwrap();
        app.send_input(b"hi\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = false;
        while Instant::now() < deadline && !got {
            for (_, ev) in app.poll_all(Instant::now()) {
                if let SessionEvent::PassThrough(bytes) = ev {
                    if bytes.windows(2).any(|w| w == b"hi") {
                        got = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(got, "expected `hi` to round-trip through cat");
        // Tell cat to exit so the test doesn't hang on shutdown.
        let _ = app.send_input(&[0x04]);
    }
```

Run:

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
```

Expected: compile error — `App::send_input` not defined.

- [ ] **Step 2: Implement `send_input`**

Add to `impl App`, after `tick_all`:

```rust
    /// Write keystroke bytes to the active tab's PTY child.
    ///
    /// # Errors
    /// Returns the tab's `io::Error` if the write fails. If there are no tabs,
    /// returns `ErrorKind::NotFound` — the caller should ensure at least one
    /// tab exists before calling.
    pub fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no active tab",
            ));
        };
        tab.send_input(bytes)
    }
```

- [ ] **Step 3: Run tests + verify**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --lib app
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 7 tests pass in app module; fmt + clippy silent.

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/app.rs
git commit -m "feat(app): App::send_input routes keystrokes to active tab"
```

---

## Task 10: `main.rs` — headless demo

**Files:**
- Modify: `crates/vibeflow/src/main.rs`

The Stage 3 binary spawns a single shell, polls for ~5 seconds, and prints any `SessionEvent`s to stderr. Useful for hand-verification: `vibeflow` will print "Working" / "Waiting" / etc. as the user runs commands. Stage 4 replaces this with the winit event loop.

- [ ] **Step 1: Replace the stub with the demo**

Replace the contents of `crates/vibeflow/src/main.rs` with:

```rust
//! Headless demo binary: spawn one scripted child that emits OSC 1338
//! transitions, observe state changes, print them.
//!
//! Stage 4 replaces this with the winit event loop and the wgpu renderer
//! plus stdin forwarding for interactive use. For Stage 3 the demo runs
//! a non-interactive script (no stdin forwarding from the demo's own
//! stdin to the PTY child) and exits cleanly when the child terminates.

use std::time::{Duration, Instant};

use vibeflow::app::App;
use vibeflow::session::SessionEvent;

/// Default child command for the demo. Emits "starting…", a Working frame,
/// sleep 2s, a Waiting frame, sleep 2s, "done", then exits. Total runtime
/// ~5 seconds. Overridable via the `VIBEFLOW_DEMO_CMD` env var.
const DEFAULT_DEMO: &str = "\
    printf 'starting...\\n'; \
    printf '\\x1b]1338;state=working;tool=demo\\x07'; \
    sleep 2; \
    printf '\\x1b]1338;state=waiting;tool=demo\\x07'; \
    sleep 2; \
    printf 'done\\n'";

fn main() -> std::io::Result<()> {
    eprintln!("vibeflow Stage 3 headless demo");
    let demo_cmd = std::env::var("VIBEFLOW_DEMO_CMD").unwrap_or_else(|_| DEFAULT_DEMO.into());

    let mut app = App::new();
    app.new_tab(&["/bin/sh", "-c", &demo_cmd])?;

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let mut died = false;
        for (idx, ev) in app.poll_all(Instant::now()) {
            match ev {
                SessionEvent::StateChanged(state) => {
                    eprintln!("[tab {idx}] state -> {state:?}");
                }
                SessionEvent::PassThrough(bytes) => {
                    // Stage 4+ pipes this into alacritty_terminal. For Stage 3,
                    // dump verbatim to our own stdout so the user sees shell
                    // output. Lossy on non-UTF-8 — fine for the demo.
                    let _ = std::io::Write::write_all(&mut std::io::stdout().lock(), &bytes);
                }
                SessionEvent::Died => {
                    eprintln!("[tab {idx}] died — exiting");
                    died = true;
                }
            }
        }
        if died {
            return Ok(());
        }
        for (idx, ev) in app.tick_all(Instant::now()) {
            if let SessionEvent::StateChanged(state) = ev {
                eprintln!("[tab {idx}] tick -> {state:?}");
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
```

- [ ] **Step 2: Build the binary and smoke-test it**

```bash
cd /path/to/vibeflow
cargo build --bin vibeflow
./target/debug/vibeflow 2>&1
```

Expected: the demo runs for about 5 seconds, prints `starting...`, then `[tab 0] state -> Working`, then (after the 2s sleep) `[tab 0] state -> Waiting`, then prints `done` and `[tab 0] died — exiting`.

If the binary hangs past 10 seconds: the child's exit isn't propagating through. Inspect by adding `eprintln!` statements in the poll loop or running with `RUST_LOG=trace` (after Stage 4 wires tracing in).

- [ ] **Step 3: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/main.rs
git commit -m "feat(vibeflow): headless demo binary that prints SessionEvents"
```

---

## Task 11: Integration test — fake AI tool drives state transitions

**Files:**
- Create: `crates/vibeflow/tests/pty_integration.rs`

A real PTY-spawned test that exercises the full pipeline: `App::new_tab` → child process emits OSC 1338 sequences → reader thread → channel → dispatcher → tracker → `SessionEvent::StateChanged`.

- [ ] **Step 1: Create the integration test file**

Write `crates/vibeflow/tests/pty_integration.rs`:

```rust
//! Integration test: fake AI tool emits OSC 1338 sequences, App observes
//! tracker state transitions through the full PTY pipeline.

use std::time::{Duration, Instant};

use vibeflow::app::App;
use vibeflow::session::SessionEvent;
use vibeflow::session::tracker::TabState;
use vibeflow_protocol::{Frame, State};

/// Render a Frame's bytes as a shell-safe `printf` argument string.
fn frame_as_printf_arg(frame: Frame) -> String {
    frame
        .to_bytes()
        .iter()
        .map(|b| format!("\\x{b:02x}"))
        .collect()
}

/// Spawn a shell that emits a known OSC 1338 sequence, sleeps to keep the
/// PTY open long enough for the test to observe events, then exits.
fn spawn_emitter_app(sequence: &str) -> App {
    let mut app = App::new();
    app.new_tab(&[
        "/bin/sh",
        "-c",
        &format!("printf '{sequence}'; sleep 5"),
    ])
    .unwrap();
    app
}

/// Poll the app for up to `max` looking for a state change on tab 0 to
/// `target`. Returns `true` if observed, `false` on timeout.
fn wait_for_state(
    app: &mut App,
    target: TabState,
    max: Duration,
) -> bool {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        for (idx, ev) in app.poll_all(Instant::now()) {
            if idx == 0 {
                if let SessionEvent::StateChanged(state) = ev {
                    if state == target {
                        return true;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn osc_1338_working_frame_drives_state_to_working() {
    let seq = frame_as_printf_arg(Frame::new(State::Working).with_tool("claude"));
    let mut app = spawn_emitter_app(&seq);
    assert!(
        wait_for_state(&mut app, TabState::Working, Duration::from_secs(5)),
        "expected tab 0 to transition to Working"
    );
    assert_eq!(app.tabs()[0].state(), TabState::Working);
}

#[test]
fn osc_1338_waiting_frame_drives_state_to_waiting() {
    let seq = frame_as_printf_arg(Frame::new(State::Waiting));
    let mut app = spawn_emitter_app(&seq);
    assert!(
        wait_for_state(&mut app, TabState::Waiting, Duration::from_secs(5)),
        "expected tab 0 to transition to Waiting"
    );
}

#[test]
fn osc_133_command_start_drives_state_to_working_via_shell_path() {
    // OSC 133;C is what shells emit when a command starts. The tracker
    // should transition to Working without any AI-tool involvement.
    let mut app = App::new();
    app.new_tab(&[
        "/bin/sh",
        "-c",
        "printf '\\x1b]133;C\\x07'; sleep 5",
    ])
    .unwrap();
    assert!(
        wait_for_state(&mut app, TabState::Working, Duration::from_secs(5)),
        "expected tab 0 to transition to Working from OSC 133;C"
    );
}

#[test]
fn child_exit_produces_died_event() {
    // Child runs `true` (exits 0 immediately). We should observe a `Died`
    // event on tab 0 within a couple of seconds.
    let mut app = App::new();
    app.new_tab(&["/bin/true"]).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut died = false;
    while Instant::now() < deadline && !died {
        for (_, ev) in app.poll_all(Instant::now()) {
            if matches!(ev, SessionEvent::Died) {
                died = true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(died, "expected SessionEvent::Died for /bin/true");
    assert!(!app.tabs()[0].is_alive());
}
```

- [ ] **Step 2: Run the integration tests**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow --test pty_integration
```

Expected: `test result: ok. 4 passed; 0 failed`.

If any test times out: the most common cause is the printf command's escape interpretation differing across `sh` implementations. `/bin/sh` on Ubuntu is `dash`, which interprets `\x1b` correctly inside `printf`. On other systems it might not. **Workaround if needed:** swap `/bin/sh -c "printf '<bytes>'"` for an explicit `printf` invocation: `["/usr/bin/env", "printf", "<bytes>"]`. Or use `python3 -c "import sys; sys.stdout.buffer.write(b'\\x1b]1338;...')"` for guaranteed byte-correct emission.

- [ ] **Step 3: Run the full test suite**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow
```

Expected: every prior unit test plus the new integration tests, all passing.

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/tests/pty_integration.rs
git commit -m "test(vibeflow): PTY integration — fake AI tool drives state transitions"
```

---

## Task 12: Final verification + tag

**Files:** none (verification + git tag)

- [ ] **Step 1: Full local CI dry-run**

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

Expected: `ALL GREEN` at the end.

- [ ] **Step 2: 60-second fuzz on the protocol parser**

```bash
cd /path/to/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: clean.

- [ ] **Step 3: Smoke-run the headless demo**

```bash
cd /path/to/vibeflow
./target/debug/vibeflow 2>&1
```

Expected: prints `starting...`, then `[tab 0] state -> Working`, sleeps 2s, then `[tab 0] state -> Waiting`, sleeps 2s, prints `done`, reports `[tab 0] died — exiting`. Total runtime ~5s.

- [ ] **Step 4: Tag the milestone**

```bash
cd /path/to/vibeflow
git tag -a stage3-pty-app-complete -m "PtySession + reader thread + headless App complete (Stage 3 of v0.1)"
git tag --list
```

- [ ] **Step 5: Surface to user**

Report:
- Number of new commits on this stage (should be 12–13).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 4 (winit + wgpu + minimal grid renderer) as the next plan.

---

## Spec coverage check

Mapping spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| Components — `PtySession` (`session/mod.rs`) | Task 2 (skeleton + reader thread), Tasks 3–5 (poll, send_input, tick) |
| Components — `App` (`app.rs`) | Task 6 (skeleton), Tasks 7–9 (poll_all, tick_all, send_input) |
| Components — `pty.rs` PTY spawn | Task 1 |
| Process & threading model — main thread + reader thread per PTY via mpsc | Tasks 2–3 (reader thread + channel) |
| Architecture — Bytes flow PTY → OscDispatcher → AiStateTracker | Task 3 (poll wires the pipeline), Task 11 (integration test) |
| Data flow A — "Claude emits 'waiting'" | Task 11 (`osc_1338_waiting_frame_drives_state_to_waiting`) |
| Data flow B — User keystroke | Task 4 (PtySession::send_input), Task 9 (App::send_input) |
| Data flow C — Plain shell, no AI | Task 11 (`osc_133_command_start_drives_state_to_working_via_shell_path`) |
| Error handling — Child process exits → mark session dead | Task 3 (poll observes channel-disconnect), Task 11 (`child_exit_produces_died_event`) |
| Error handling — PTY spawn fails | Task 1 (return io::Error from spawn_pty) |
| Error handling — Reader thread errors | Task 2 (reader thread exits cleanly on Read error) |

**Out of scope for this plan (deferred to later stages):**
- winit window + event loop — Stage 4.
- alacritty_terminal grid wiring — Stage 4 (consumes `SessionEvent::PassThrough`).
- wgpu rendering — Stage 4–5.
- Foreground-process detection driving `set_heuristic_active(true)` — Stage 6 (needs OS-specific `procfs` polling on Linux).
- Resize / SIGWINCH — Stage 6 (needs window dimensions).
- Configuration parsing (TOML loading of TrackerConfig) — Stage 8.

## Self-review

- **Spec coverage:** every Stage 3-relevant spec requirement has a task. Stages 4+ items are explicitly listed as out of scope.
- **Placeholder scan:** no `TBD`/`TODO`/`implement later`/`similar to` patterns. Each step has actual code or actual commands.
- **Type consistency check:**
  - `PtyHandles` (Task 1) used identically in `PtySession::spawn` (Task 2).
  - `SessionEvent` (Task 2) used identically in poll (Task 3), tick (Task 5), App::poll_all (Task 7), App::tick_all (Task 8), main.rs (Task 10), integration tests (Task 11).
  - `App` methods agree on `&mut self` signature, `Instant` argument order, `&[&str]` argv shape.
  - `TrackerConfig` flows from `App::new` → `App::new_tab` → `PtySession::spawn` → `AiStateTracker::new`.
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy step.
- **Threading-model discipline:** the reader thread is spawned in `PtySession::spawn` (Task 2) and joined in `Drop`. No `Arc<Mutex<…>>` on the tracker. The mpsc channel is the only cross-thread communication path. Matches spec exactly.
- **Forward-declared item handling:** `SessionEvent::Died` is introduced in Task 2 with the `_unused_session_event_silences_dead_code` test in Task 6 as a temporary suppression; that test is removed in Task 8 once `poll_all` makes the variant naturally reachable. Documented inline.
