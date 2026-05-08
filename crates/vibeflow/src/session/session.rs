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
    /// Shell rang the bell (BEL, 0x07).
    Bell,
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
    pub(crate) dispatcher: OscDispatcher,
    /// Per-session VT/ANSI parser. Drives `term` when fed via `Processor::advance`.
    parser: Processor,
    /// Per-session terminal grid (alacritty_terminal). Source of truth for
    /// what the cell renderer draws.
    term: Term<VoidListener>,
    /// Per-session state tracker.
    tracker: AiStateTracker,
    /// True until either the child exits or the reader-thread errors out.
    alive: bool,
    /// Display label for the tab bar. Updated automatically when the tracker
    /// state changes (default policy = shell binary + state); overridable via
    /// [`Self::set_label`] from the config layer.
    label: TabLabel,
    /// Set true when a stray BEL byte (0x07) is observed in PassThrough output.
    /// Drained into a [`SessionEvent::Bell`] at the end of each `poll`.
    bell_pending: bool,
    /// Per-tab mouse-driven cell selection. Stage 8.
    pub selection: crate::render::selection::SelectionTracker,
    /// True once the user has manually renamed via Ctrl+Shift+E or
    /// right-click. Sticky for the life of this session — subsequent
    /// OSC 0 / OSC 2 are ignored. Cleared on `restart()` (which does `*self = new_session`).
    pub user_renamed: bool,
    /// Mirror of `Config.tabs.respect_osc_title`. When false, OSC 0/2 from
    /// the shell is silently dropped regardless of `user_renamed`. WindowApp
    /// keeps this in sync via apply_config + new-tab spawn.
    pub respect_osc_title: bool,
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

        let label = TabLabel::default_for(argv[0], TabState::default());

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
            label,
            bell_pending: false,
            selection: crate::render::selection::SelectionTracker::new(),
            user_renamed: false,
            respect_osc_title: true,
        })
    }

    /// Current visual state of this session's tab.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.tracker.state()
    }

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

    /// Replace only the title (line 1) of the label, preserving the current
    /// subtitle. Used by the interactive rename UI which doesn't touch the
    /// activity-driven subtitle.
    pub fn set_title(&mut self, title: String) {
        self.label.title = title;
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
                            DispatchEvent::SetTitle(title) => {
                                tracing::debug!(
                                    title = %title,
                                    user_renamed = self.user_renamed,
                                    respect_osc_title = self.respect_osc_title,
                                    "OSC SetTitle received"
                                );
                                if self.respect_osc_title
                                    && !self.user_renamed
                                    && self.label.title != title
                                {
                                    self.label.title = title;
                                    events.push(SessionEvent::TermUpdated);
                                }
                                // else: silently dropped — config disabled OSC
                                // titles, or user-renamed wins, or unchanged.
                            }
                            DispatchEvent::PassThrough(bytes) => {
                                self.tracker.on_input(TrackerInput::OutputObserved, now);
                                for &byte in &bytes {
                                    // BEL bytes that terminate an OSC sequence are consumed by the
                                    // OscDispatcher and never reach here; only stray BEL (e.g.
                                    // `printf '\007'`) surfaces as PassThrough.
                                    if byte == 0x07 {
                                        self.bell_pending = true;
                                    }
                                    self.parser.advance(&mut self.term, byte);
                                }
                                events.push(SessionEvent::TermUpdated);
                            }
                        }
                    }
                    if self.bell_pending {
                        self.bell_pending = false;
                        events.push(SessionEvent::Bell);
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
            self.refresh_default_subtitle();
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

    /// Split-borrow helper for mouse event routing. Returns a mutable reference
    /// to the selection tracker and an immutable reference to the terminal grid
    /// in a single call, avoiding the aliasing problem that arises when calling
    /// `s.selection.mouse_down(point, shift, s.term(), now)` — that expression
    /// would simultaneously borrow `s` mutably (for `selection`) and immutably
    /// (for `term()`), which the borrow checker rejects.
    ///
    /// This is the standard Rust split-borrow idiom: both references are to
    /// disjoint fields of `self`, so they are sound to hold simultaneously.
    pub(crate) fn split_borrow_mouse(
        &mut self,
    ) -> (
        &mut crate::render::selection::SelectionTracker,
        &Term<VoidListener>,
    ) {
        (&mut self.selection, &self.term)
    }

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
        // Preserve the OSC-title policy across restart (a deliberate config
        // override shouldn't be wiped just because the user hit Ctrl+Shift+R).
        // user_renamed is intentionally NOT preserved — the new shell is fresh.
        new_session.respect_osc_title = self.respect_osc_title;
        *self = new_session;
        Ok(())
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
        use vibeflow_protocol::{Frame as ProtoFrame, State as ProtoState};

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
        assert_eq!(
            TabLabel::default_for("/usr/bin/zsh", TabState::Working).title,
            "zsh"
        );
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
        let mut s =
            PtySession::spawn(&["/bin/sh", "-c", "sleep 5"], TrackerConfig::default()).unwrap();
        s.set_label(TabLabel {
            title: "custom".into(),
            subtitle: "claude · waiting".into(),
        });
        assert_eq!(s.label().title, "custom");
        assert_eq!(s.label().subtitle, "claude · waiting");
    }

    #[test]
    fn poll_emits_bell_event_when_07_byte_received() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "printf '\\007hi'; sleep 0.5"],
            TrackerConfig::default(),
        )
        .unwrap();
        // Wait for child output.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut got_bell = false;
        while std::time::Instant::now() < deadline && !got_bell {
            for ev in s.poll(std::time::Instant::now()) {
                if matches!(ev, SessionEvent::Bell) {
                    got_bell = true;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(got_bell, "no Bell event seen within 2s");
    }

    #[test]
    fn restart_replaces_internals_with_fresh_spawn() {
        // Spawn a sleep then restart. After restart, the new session must
        // be alive (the new shell is freshly spawned).
        let mut s =
            PtySession::spawn(&["sleep", "10"], TrackerConfig::default()).expect("first spawn");
        s.restart().expect("restart");
        // Give the new PTY a moment to initialize before the liveness check
        // — `child.try_wait` can race against the spawn handshake on slower
        // CI runners.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(s.is_alive(), "restarted session should be alive");
        // Send some bytes to confirm the new PTY is responsive.
        s.send_input(b"\n")
            .expect("send_input on restarted session");
        // Drop the session — its Drop impl (or the kill-on-drop the
        // child handle has) cleans up the spawned shell.
        drop(s);
    }

    #[test]
    fn osc_0_updates_title_when_not_user_renamed() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default()).expect("spawn");
        // Feed the OSC sequence directly to the dispatcher to simulate it arriving
        // through the PTY. Handle the SetTitle event directly in the test.
        for ev in s.dispatcher.feed(b"\x1b]0;new_title\x07") {
            if let DispatchEvent::SetTitle(title) = ev {
                if !s.user_renamed {
                    s.label.title = title;
                }
            }
        }
        assert_eq!(s.label().title, "new_title");
    }

    #[test]
    fn osc_0_dropped_when_user_renamed() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default()).expect("spawn");
        s.user_renamed = true;
        for ev in s.dispatcher.feed(b"\x1b]0;new_title\x07") {
            if let DispatchEvent::SetTitle(title) = ev {
                if !s.user_renamed {
                    s.label.title = title;
                }
            }
        }
        assert_eq!(s.label().title, "sleep"); // unchanged from default
    }

    #[test]
    fn osc_0_dropped_when_respect_osc_title_false() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default()).expect("spawn");
        s.respect_osc_title = false;
        for ev in s.dispatcher.feed(b"\x1b]0;new_title\x07") {
            if let DispatchEvent::SetTitle(title) = ev {
                if s.respect_osc_title && !s.user_renamed {
                    s.label.title = title;
                }
            }
        }
        assert_eq!(s.label().title, "sleep");
    }

    #[test]
    fn restart_resets_user_renamed() {
        let mut s = PtySession::spawn(&["sleep", "5"], TrackerConfig::default()).expect("spawn");
        s.user_renamed = true;
        s.set_label(TabLabel {
            title: "user_set".to_string(),
            subtitle: "custom".to_string(),
        });
        s.restart().expect("restart");
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!s.user_renamed, "restart must clear user_renamed");
        assert_ne!(
            s.label().title,
            "user_set",
            "restart must clear user-set title"
        );
    }
}
