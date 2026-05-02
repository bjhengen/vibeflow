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

    /// Current visual state of this session's tab.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.tracker.state()
    }

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

    /// Whether the child is still running and the reader thread alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Write keystroke bytes to the PTY master. The child sees these as input
    /// on its stdin.
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

    /// Toggle the Tier 3 heuristic-silence inference. The App calls this when
    /// the foreground process matches the configured AI-tool list.
    pub fn set_heuristic_active(&mut self, active: bool) {
        self.tracker.set_heuristic_active(active);
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
        let s = PtySession::spawn(&["/bin/sh", "-c", "printf hello"], TrackerConfig::default())
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

    use vibeflow_protocol::{Frame as ProtoFrame, State as ProtoState};

    #[test]
    fn poll_routes_osc_1338_through_dispatcher_and_tracker() {
        // Spawn a child that prints exactly one OSC 1338 sequence, then exits.
        // The session's poll() should observe a state change to Working.
        let bytes = ProtoFrame::new(ProtoState::Working).to_bytes();
        let bytes_repr = bytes
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                &format!("import sys; sys.stdout.buffer.write(bytes([{bytes_repr}]))"),
            ],
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
    fn tick_fires_stale_state_timeout() {
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 60"], TrackerConfig::default()).unwrap();
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
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
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

    #[test]
    fn resize_does_not_error_on_a_live_session() {
        // We don't assert anything about the child observing the new size — the
        // ioctl semantics are portable-pty's responsibility. We just verify the
        // call succeeds end-to-end (no Mutex poisoning, no consumed-master
        // panic, no Result::Err path).
        let s = PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
        s.resize(40, 100).unwrap();
        // Issue a second resize to verify it's not a one-shot.
        s.resize(24, 80).unwrap();
    }
}
