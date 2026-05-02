//! `PtySession` — one tab's PTY child, reader thread, OSC dispatcher, and AI-state
//! tracker, and `alacritty_terminal::Term`. All driven from the main thread
//! via a single-producer single-consumer channel.

use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::Child;

use crate::session::osc::{DispatchEvent, OscDispatcher};
use crate::session::pty::{spawn_pty, PtyHandles};
use crate::session::tracker::{AiStateTracker, TabState, TrackerConfig, TrackerInput};

/// Default tab size when the session is first spawned. The window size in
/// `WindowApp::resumed` calls `App::resize_all` shortly after, which calls
/// `PtySession::resize` and updates both the PTY and `Term`.
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

/// Public event type the `App` observes from a session, beyond just the
/// underlying [`DispatchEvent`]. `Died` lets the App detect when the child
/// exits and the reader thread has finished. `TermUpdated` is the redraw
/// trigger — bytes were consumed by the per-session [`Term`], so the grid
/// changed and the renderer should refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// State of the per-session tracker just changed to this value.
    StateChanged(TabState),
    /// Bytes were consumed by [`Term`]; the grid changed. The renderer reads
    /// the current grid via [`PtySession::term`] / [`crate::app::App::active_term`].
    TermUpdated,
    /// The child exited or the reader thread terminated. After this event,
    /// `is_alive()` returns false and further `poll()` calls produce nothing.
    Died,
}

/// One terminal tab's per-session machinery.
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
    /// Per-session VT/ANSI parser. Drives `term` when fed via `Processor::advance`.
    parser: Processor,
    /// Per-session terminal grid (alacritty_terminal). Source of truth for
    /// what the cell renderer draws.
    term: Term<VoidListener>,
    /// Per-session state tracker.
    tracker: AiStateTracker,
    /// True until either the child exits or the reader-thread errors out.
    alive: bool,
}

impl PtySession {
    /// Spawn a child via the given `argv` on a fresh pseudoterminal and start
    /// the reader thread.
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

        let term_size = TermSize::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize);
        let term = Term::new(TermConfig::default(), &term_size, VoidListener);

        Ok(Self {
            rx,
            writer,
            master,
            child,
            reader_thread: Some(reader_thread),
            dispatcher: OscDispatcher::new(),
            parser: Processor::new(),
            term,
            tracker: AiStateTracker::new(config),
            alive: true,
        })
    }

    /// Current visual state of this session's tab.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.tracker.state()
    }

    /// Drain every pending byte chunk off the reader channel, run each through
    /// the dispatcher, route resulting events into the tracker AND the per-session
    /// `Term`, and return the public-facing [`SessionEvent`]s for the App.
    /// Non-blocking — returns immediately if the channel is empty.
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
                                // Feed bytes through the VT parser into Term. This is
                                // where the grid actually updates.
                                for &byte in &bytes {
                                    self.parser.advance(&mut self.term, byte);
                                }
                                events.push(SessionEvent::TermUpdated);
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
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

    /// Write keystroke bytes to the PTY master.
    ///
    /// # Errors
    /// Propagates any underlying `io::Error` from the writer.
    pub fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

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

    /// Resize the PTY to `rows` rows × `cols` cols, AND resize the per-session
    /// `Term` so the grid layout matches.
    ///
    /// # Errors
    /// Wraps `portable_pty`'s typed error via `io::Error::other`.
    pub fn resize(&mut self, rows: u16, cols: u16) -> std::io::Result<()> {
        self.master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)?;
        self.term
            .resize(TermSize::new(cols as usize, rows as usize));
        Ok(())
    }

    /// Whether the child is still running and the reader thread alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Read-only access to the per-session `Term` for rendering.
    #[must_use]
    pub fn term(&self) -> &Term<VoidListener> {
        &self.term
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
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
    fn poll_routes_osc_1338_through_dispatcher_and_tracker() {
        use crate::session::tracker::TabState;

        // Spawn a child that outputs OSC-1338 with a state change.
        // After poll, the tracker should have state Waiting.
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "
import sys
# Emit an OSC-1338 frame indicating the tool is waiting for input.
osc_frame = b'\\x1b]1338;state=waiting\\x07'
sys.stdout.buffer.write(osc_frame)
sys.stdout.flush()
",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_waiting = false;
        while Instant::now() < deadline && !saw_waiting {
            for ev in s.poll(Instant::now()) {
                if matches!(ev, SessionEvent::StateChanged(TabState::Waiting)) {
                    saw_waiting = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(saw_waiting, "expected Waiting state from OSC-1338");
    }

    #[test]
    fn poll_receives_output_and_emits_term_updated() {
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys, time; sys.stdout.buffer.write(b'hello'); sys.stdout.flush(); time.sleep(2)",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_term_updated = false;
        while Instant::now() < deadline && !saw_term_updated {
            for ev in s.poll(Instant::now()) {
                if matches!(ev, SessionEvent::TermUpdated) {
                    saw_term_updated = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(saw_term_updated, "expected TermUpdated from output");
    }

    #[test]
    fn tick_does_not_fire_within_timeout_windows() {
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
        // Default config: stale_state 30s, heuristic_silence 4s — neither
        // fires within 1s of spawn.
        let evs = s.tick(Instant::now() + Duration::from_secs(1));
        assert_eq!(evs, vec![]);
    }

    #[test]
    fn set_heuristic_active_toggles_tier_3() {
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 10"], TrackerConfig::default()).unwrap();
        // Just verify the call doesn't panic — tracker's internal logic is
        // tested in tracker tests.
        s.set_heuristic_active(true);
        s.set_heuristic_active(false);
    }

    #[test]
    fn poll_clears_reader_channel_non_blocking() {
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys; sys.stdout.buffer.write(b'hello'); sys.stdout.flush()",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut got_bytes = false;
        while Instant::now() < deadline && !got_bytes {
            let evs = s.poll(Instant::now());
            if !evs.is_empty() {
                got_bytes = true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(got_bytes, "expected some output from python");
    }

    #[test]
    fn send_input_writes_to_pty_master() {
        let mut s = PtySession::spawn(&["/bin/cat"], TrackerConfig::default()).unwrap();
        // Just verify the call doesn't error — the PTY driver will echo it back.
        s.send_input(b"hello\n").unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn resize_does_not_error_on_a_live_session() {
        // We don't assert anything about the child observing the new size — the
        // ioctl semantics are portable-pty's responsibility. We just verify the
        // call succeeds end-to-end (no Mutex poisoning, no consumed-master
        // panic, no Result::Err path).
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
        s.resize(40, 100).unwrap();
        // Issue a second resize to verify it's not a one-shot.
        s.resize(24, 80).unwrap();
    }

    #[test]
    fn term_consumes_bytes_during_poll() {
        // Spawn a child that writes a known string. After poll, Term's grid
        // should contain the characters.
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys, time; sys.stdout.buffer.write(b'hello\\n'); sys.stdout.flush(); time.sleep(2)",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() >= deadline {
                panic!("never observed Term contents");
            }
            let _events = s.poll(Instant::now());
            // Read the first row of Term and look for "hello".
            let row_text: String = s
                .term()
                .renderable_content()
                .display_iter
                .filter(|i| i.point.line.0 == 0)
                .map(|i| i.cell.c)
                .collect();
            if row_text.contains("hello") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn poll_emits_term_updated_when_bytes_arrive() {
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys, time; sys.stdout.buffer.write(b'hi'); sys.stdout.flush(); time.sleep(2)",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_term_updated = false;
        while Instant::now() < deadline && !saw_term_updated {
            for ev in s.poll(Instant::now()) {
                if matches!(ev, SessionEvent::TermUpdated) {
                    saw_term_updated = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(saw_term_updated, "expected at least one TermUpdated event");
    }
}
