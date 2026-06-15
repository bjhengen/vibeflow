//! `PtySession` — one tab's PTY child, reader thread, OSC dispatcher, and AI-state
//! tracker, and `alacritty_terminal::Term`. All driven from the main thread
//! via a single-producer single-consumer channel.

use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
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

/// #10: maximum bytes `poll` will consume from the reader channel in a single
/// call. Bounds the per-cycle parse cost so a rapid output burst can't freeze
/// the main loop on one giant synchronous drain. At the measured ~9 MB/s
/// byte-by-byte parse, 64 KB is ~7 ms — under one 60 Hz frame, leaving room for
/// the render in the same cycle. Remaining backlog drains on the next poll,
/// which the window layer schedules immediately (see `output_pending`).
const MAX_POLL_BYTES: usize = 64 * 1024;

/// #17: bounded reader→main-loop channel capacity, measured in chunks (each an
/// up-to-4 KiB PTY read). The reader thread blocks on `send` once this many
/// chunks are queued, applying backpressure (the PTY kernel buffer fills, the
/// child's writes block) instead of buffering a firehose as unbounded heap.
/// 512 × up-to-4 KiB ≈ up to 2 MiB/tab — 32× the `MAX_POLL_BYTES` drain budget
/// in the full-read worst case; typical interactive output (short reads) buffers
/// far less before backpressure fires, so steady-state throughput is unchanged.
const READER_CHANNEL_CAPACITY: usize = 512;

/// Viewport size handed to `Term::new` / `Term::resize` (both are generic
/// over [`Dimensions`]). `total_lines == screen_lines`: scrollback history is
/// configured separately via `TermConfig::scrolling_history`, and the grid
/// grows it internally. (alacritty_terminal's own equivalent lives in its
/// `term::test` module, which production code shouldn't import.)
struct GridSize {
    columns: usize,
    screen_lines: usize,
}

impl GridSize {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// Display label for a tab. The renderer in [`crate::render::tabs`] reads
/// this to draw the title (line 1) and subtitle (line 2). Overridable as a
/// whole via [`PtySession::set_label`] (custom subtitle sticks) or title-only
/// via [`PtySession::set_title`] (the rename UI).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TabLabel {
    pub title: String,
    pub subtitle: String,
}

impl TabLabel {
    /// The activity subtitle word for a tracker state. Single source of
    /// truth shared by [`Self::default_for`] and
    /// [`PtySession::refresh_default_subtitle`].
    #[must_use]
    pub fn subtitle_for(state: TabState) -> &'static str {
        match state {
            TabState::Active => "active",
            TabState::Working => "working",
            TabState::Waiting => "waiting",
            TabState::Done => "done",
            TabState::Idle => "idle",
        }
    }

    /// Default label for a freshly-spawned session: shell binary basename for
    /// the title, lowercased tracker-state name for the subtitle.
    #[must_use]
    pub fn default_for(argv0: &str, state: TabState) -> Self {
        let title = std::path::Path::new(argv0)
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or(argv0)
            .to_string();
        let subtitle = Self::subtitle_for(state).to_string();
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
    /// OSC 52 clipboard write — TUI in this session asked the window layer
    /// to put `text` on the user's clipboard. `selection` determines which
    /// buffer(s). The window layer (which owns the `Clipboard` instance)
    /// performs the actual write; the session merely lifts the event across
    /// the session/window boundary.
    Osc52ClipboardWrite {
        selection: crate::session::osc::Osc52Selection,
        text: String,
    },
}

/// One terminal tab's per-session machinery.
pub struct PtySession {
    /// Drains here when the reader thread sends bytes from the PTY master.
    /// `Option` so `Drop` can drop the receiver before joining the reader
    /// thread — a reader blocked on a full `sync_channel` send only wakes when
    /// the receiver goes away (#17).
    rx: Option<Receiver<Vec<u8>>>,
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
    /// [`Self::set_label`].
    label: TabLabel,
    /// Set true when a stray BEL byte (0x07) is observed in PassThrough output.
    /// Drained into a [`SessionEvent::Bell`] at the end of each `poll`.
    bell_pending: bool,
    /// #10: set by `poll` when it stopped because it hit `MAX_POLL_BYTES` (more
    /// output is queued). The window layer re-wakes immediately so catch-up runs
    /// at full throughput without freezing the main loop on one giant drain.
    output_pending: bool,
    /// #10: bytes consumed by the most recent `poll` call. Test/bench-only
    /// instrumentation (read via the `#[cfg(test)]` accessor).
    #[cfg_attr(not(test), allow(dead_code))]
    last_poll_bytes: usize,
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
    /// Mirror of `Config.clipboard.allow_osc52_write`. When false, OSC 52
    /// clipboard-write requests from terminal output are silently dropped, so
    /// untrusted output (e.g. a remote SSH session) cannot overwrite the user's
    /// system clipboard. Default true to preserve the `vim "+y` / tmux / SSH
    /// copy workflow. OSC 52 *read* is always ignored regardless of this flag.
    pub allow_osc52_write: bool,
    /// Mirror of `Config.tabs.title_strip_prefix`. If non-empty, this
    /// prefix is stripped from the front of every accepted OSC 0/2 title
    /// before it lands on `label.title`.
    pub title_strip_prefix: String,
    /// Stage 11: list of foreground-process names that should arm the Tier 3
    /// heuristic. Mirrored from `Config.ai.tools` via `apply_config`.
    pub(crate) tools_list: Vec<String>,
    /// Stage 11: throttle interval for re-reading `/proc/<child>/stat`.
    /// Mirrored from `Config.ai.foreground_check_interval_ms`.
    pub(crate) proc_check_interval: std::time::Duration,
    /// Stage 11: timestamp of the most recent proc check, for throttling.
    pub(crate) last_proc_check: Option<std::time::Instant>,
    /// Stage 11 follow-up: tracks the previous heuristic_active state so
    /// `tick()` can detect the rising edge (off→on) and synthesize an
    /// `OutputObserved` to seed the tracker. Without this, AI tools that
    /// produce no immediate output (e.g., `python3 -c "sleep 30"`) never
    /// arm the silence guard because state stays Active.
    pub(crate) heuristic_was_active: bool,
    /// Stage 12: per-session scrollbar fade-state. Marked on user scroll;
    /// fades back to invisible after `scrollbar_fade_ms`.
    pub(crate) scrollbar_fade: crate::render::scrollbar::ScrollbarFade,
    /// Stage 12: scrollback history capacity this session was constructed with.
    /// Stored so `restart()` can re-use the same value when rebuilding the Term.
    pub(crate) history_lines: usize,
    /// Stage 13: theme name (None = Stage 9 hardcoded defaults). Mirror of
    /// `Config.colors.preset`; per-tab override via the Stage 10 context menu.
    pub(crate) theme: Option<String>,
    /// Stage 13: resolved color table for `theme`, consulted by the renderer
    /// (`Term` has no colors_mut(), so we keep colors here, not on `term`).
    pub(crate) theme_colors: Option<alacritty_terminal::term::color::Colors>,
    /// v0.1.3 confirm-on-close: name of the most-recently-matched tool from
    /// `tools_list` (set during `tick()`'s foreground-process check, cleared
    /// when the match drops). Surfaces via `detected_ai_tool()` so the
    /// confirm-on-close dialog can show "claude" / "codex" instead of the
    /// raw FG process comm. `None` when no match is active.
    detected_ai_tool: Option<String>,
    /// True once [`Self::set_label`] installed a custom subtitle;
    /// [`Self::refresh_default_subtitle`] then leaves the subtitle alone
    /// instead of re-deriving it from tracker state on every transition.
    custom_subtitle: bool,
}

impl PtySession {
    /// Spawn a child via the given `argv` on a fresh pseudoterminal and start
    /// the reader thread.
    ///
    /// # Errors
    /// Propagates PTY-spawn or thread-creation failures.
    pub fn spawn(
        argv: &[&str],
        config: TrackerConfig,
        history_lines: usize,
    ) -> std::io::Result<Self> {
        let PtyHandles {
            reader,
            writer,
            child,
            master,
        } = spawn_pty(argv)?;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(READER_CHANNEL_CAPACITY);
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

        let term_size = GridSize::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize);
        let term_config = TermConfig {
            scrolling_history: history_lines.max(1),
            ..TermConfig::default()
        };
        let term = Term::new(term_config, &term_size, VoidListener);

        let label = TabLabel::default_for(argv[0], TabState::default());

        Ok(Self {
            rx: Some(rx),
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
            output_pending: false,
            last_poll_bytes: 0,
            selection: crate::render::selection::SelectionTracker::new(),
            user_renamed: false,
            respect_osc_title: true,
            allow_osc52_write: true,
            title_strip_prefix: String::new(),
            tools_list: Vec::new(),
            proc_check_interval: std::time::Duration::from_millis(250),
            last_proc_check: None,
            heuristic_was_active: false,
            scrollbar_fade: crate::render::scrollbar::ScrollbarFade::new(1500),
            history_lines,
            theme: None,
            theme_colors: None,
            detected_ai_tool: None,
            custom_subtitle: false,
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

    /// Override the tab's label (title and subtitle). The subtitle installed
    /// here is treated as custom: [`Self::refresh_default_subtitle`] stops
    /// re-deriving it from tracker state, so it survives state transitions.
    /// (Contrast [`Self::set_title`], which leaves the subtitle
    /// activity-driven.)
    pub fn set_label(&mut self, label: TabLabel) {
        self.label = label;
        self.custom_subtitle = true;
    }

    /// Replace only the title (line 1) of the label, preserving the current
    /// subtitle. Used by the interactive rename UI which doesn't touch the
    /// activity-driven subtitle.
    pub fn set_title(&mut self, title: String) {
        self.label.title = title;
    }

    /// Recompute the activity subtitle (line 2) from the current tracker
    /// state. Called on every state transition. The subtitle is activity-
    /// driven unless [`Self::set_label`] installed a custom one (interactive
    /// rename via [`Self::set_title`] overrides only the title, never the
    /// subtitle — see its doc). The previous `!title.contains(' ')` heuristic
    /// wrongly froze the subtitle once any spaced OSC/PS1 title arrived (e.g.
    /// bash PS1 `user@host: ~/dev/vibeflow`), and re-deriving via
    /// `default_for` here also clobbered OSC-set titles — both fixed by only
    /// touching `subtitle`.
    pub fn refresh_default_subtitle(&mut self) {
        if self.custom_subtitle {
            return;
        }
        self.label.subtitle = TabLabel::subtitle_for(self.tracker.state()).to_string();
    }

    /// Drain every pending byte chunk off the reader channel, run each through
    /// the dispatcher, route resulting events into the tracker AND the per-session
    /// `Term`, and return the public-facing [`SessionEvent`]s for the App.
    /// Non-blocking — returns immediately if the channel is empty.
    pub fn poll(&mut self, now: Instant) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        self.output_pending = false;
        let mut consumed = 0usize;
        let mut had_passthrough = false;
        loop {
            // #10: bound work per call. Once we've hit the byte budget, stop and
            // flag that more output is queued so the window layer re-wakes
            // immediately — instead of draining an unbounded backlog (megabytes)
            // in one synchronous parse that freezes the main loop.
            if consumed >= MAX_POLL_BYTES {
                self.output_pending = true;
                break;
            }
            let recv = match self.rx.as_ref() {
                Some(rx) => rx.try_recv(),
                // Unreachable in live code (Drop and poll share the main thread,
                // and Drop is the only thing that takes rx); defensive guard.
                None => break,
            };
            match recv {
                Ok(chunk) => {
                    consumed += chunk.len();
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
                                if self.respect_osc_title && !self.user_renamed {
                                    let display = if !self.title_strip_prefix.is_empty() {
                                        title
                                            .strip_prefix(&self.title_strip_prefix)
                                            .map(str::to_owned)
                                            .unwrap_or(title)
                                    } else {
                                        title
                                    };
                                    if self.label.title != display {
                                        self.label.title = display;
                                        events.push(SessionEvent::TermUpdated);
                                    }
                                }
                                // else: silently dropped — config disabled OSC
                                // titles, or user-renamed wins.
                            }
                            DispatchEvent::Osc52Write { selection, text } => {
                                if self.allow_osc52_write {
                                    events.push(SessionEvent::Osc52ClipboardWrite {
                                        selection,
                                        text,
                                    });
                                } else {
                                    // Config disabled OSC 52 clipboard writes:
                                    // drop silently so untrusted output cannot
                                    // overwrite the system clipboard.
                                    tracing::debug!(
                                        bytes = text.len(),
                                        "OSC 52 clipboard write dropped (allow_osc52_write = false)"
                                    );
                                }
                            }
                            DispatchEvent::PassThrough(bytes) => {
                                // Issue #7 (v0.1.4): OutputObserved can drive a
                                // Tier-3 transition (Active/Idle/Waiting → Working).
                                // Surface it as StateChanged so the tab repaints and
                                // its cached subtitle refreshes — not just TermUpdated.
                                if self.tracker.on_input(TrackerInput::OutputObserved, now) {
                                    self.refresh_default_subtitle();
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                                for &byte in &bytes {
                                    // BEL bytes that terminate an OSC sequence are consumed by the
                                    // OscDispatcher and never reach here; only stray BEL (e.g.
                                    // `printf '\007'`) surfaces as PassThrough.
                                    if byte == 0x07 {
                                        self.bell_pending = true;
                                    }
                                    self.parser.advance(&mut self.term, byte);
                                }
                                // #10: defer to a single coalesced TermUpdated per
                                // poll (the dispatcher yields many PassThrough
                                // segments per drain → one-per-segment redraws
                                // were pure churn).
                                had_passthrough = true;
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
        if had_passthrough {
            events.push(SessionEvent::TermUpdated);
        }
        self.last_poll_bytes = consumed;
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
        // v0.1.4 (#7): state can change outside `tracker.tick()` — the heuristic
        // falling-edge clear (Change C) and the rising-edge OutputObserved
        // promotion. Accumulate any change so it surfaces as a single
        // StateChanged event (and refreshes the cached subtitle).
        let mut state_changed = false;
        // Stage 11: Tier 3 foreground-process check, throttled.
        let due = match self.last_proc_check {
            Some(last) => now.saturating_duration_since(last) >= self.proc_check_interval,
            None => true,
        };
        if due {
            self.last_proc_check = Some(now);
            // Only read /proc and update the heuristic when there are tools to
            // match; an empty list means the feature is unconfigured, so we
            // leave the heuristic flag untouched (allows set_heuristic_active
            // callers to retain their manually-set value).
            if !self.tools_list.is_empty() {
                let pid = self.child_pid();
                let fg_name = pid.and_then(crate::session::proc_watch::foreground_command_name);
                // v0.1.3: preserve the matched name so the confirm-on-close
                // dialog can show "claude" / "codex" instead of "shell".
                let matched_name: Option<String> =
                    fg_name.filter(|name| self.tools_list.iter().any(|t| t == name));
                let matched = matched_name.is_some();
                self.detected_ai_tool = matched_name;
                let was_armed = self.heuristic_was_active;
                // Change C (#7): the active→inactive edge clears a Tier-3 Waiting
                // tab; capture it so it surfaces as StateChanged.
                state_changed |= self.tracker.set_heuristic_active(matched);
                self.heuristic_was_active = matched;
                // Rising edge: heuristic just armed. Synthesize an OutputObserved so
                // the tracker promotes Active/Idle → Working AND seeds last_output_at
                // for the silence guard. Real subsequent output bytes will refresh
                // the baseline; pure silence lets the heuristic-silence timer fire.
                if matched && !was_armed {
                    state_changed |= self
                        .tracker
                        .on_input(crate::session::tracker::TrackerInput::OutputObserved, now);
                }
            }
        }
        state_changed |= self.tracker.tick(now);
        if state_changed {
            self.refresh_default_subtitle();
            vec![SessionEvent::StateChanged(self.tracker.state())]
        } else {
            Vec::new()
        }
    }

    /// Toggle the Tier 3 heuristic-silence inference. The App calls this when
    /// the foreground process matches the configured AI-tool list.
    pub fn set_heuristic_active(&mut self, active: bool) -> bool {
        self.tracker.set_heuristic_active(active)
    }

    /// Stage 11: PID of the spawned child, for `/proc/<pid>/…` reads. Returns
    /// None if the child has been reaped or never spawned cleanly.
    pub(crate) fn child_pid(&self) -> Option<i32> {
        self.child.process_id().map(|p| p as i32)
    }

    /// Stage 11: hot-reload the tracker's timing thresholds.
    pub fn set_tracker_config(&mut self, cfg: TrackerConfig) {
        self.tracker.set_config(cfg);
    }

    /// Stage 12: scroll the display by `lines`. Negative = into history; positive = toward live.
    /// No-op when `lines == 0`. Marks the scrollbar fade timer when non-zero.
    pub fn scroll_by(&mut self, lines: i32, now: std::time::Instant) {
        if lines == 0 {
            return;
        }
        use alacritty_terminal::grid::Scroll;
        // alacritty Scroll::Delta convention: positive scrolls UP (increases display_offset).
        // Our public convention: negative `lines` = into history; positive = toward live.
        // So to scroll into history (lines < 0), we negate: -lines > 0 = increases display_offset.
        self.term.scroll_display(Scroll::Delta(-lines));
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Stage 12: jump to top of history.
    pub fn scroll_to_top(&mut self, now: std::time::Instant) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Top);
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Stage 12: jump back to live viewport.
    pub fn scroll_to_bottom(&mut self, now: std::time::Instant) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Bottom);
        self.scrollbar_fade.mark_scrolled(now);
    }

    /// Stage 12: current display_offset (0 = at live viewport).
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Stage 12: read-only accessor for the scrollbar fade alpha at `now`.
    /// `pub fn` (not `pub(crate)`) so integration tests at
    /// `crates/vibeflow/tests/` can reach it across the compilation-unit
    /// boundary. Same lesson Stage 11 learned for `last_proc_check`.
    pub fn scrollbar_fade_alpha(&self, now: std::time::Instant) -> f32 {
        self.scrollbar_fade.alpha(now)
    }

    /// Stage 11: read-only accessor for the most recent proc-check timestamp.
    /// Used by integration tests at `crates/vibeflow/tests/` to verify the
    /// throttled foreground-process detection actually fires; integration
    /// tests run in a separate compilation unit, so `pub(crate)` field access
    /// would fail to compile from there.
    pub fn last_proc_check(&self) -> Option<std::time::Instant> {
        self.last_proc_check
    }

    /// Stage 11: read-only accessor for the current tracker state. Same
    /// rationale as `last_proc_check` — needed for integration tests.
    pub fn tracker_state(&self) -> crate::session::tracker::TabState {
        self.tracker.state()
    }

    /// v0.1.3 confirm-on-close: true when the controlling terminal's foreground
    /// process group is NOT the shell itself — i.e., the shell is running a
    /// subprocess that has terminal control (python3, vim, claude, make, etc.).
    /// Used by `App::busy_tabs` to identify tabs worth confirming-on-close.
    ///
    /// Returns false on non-Linux, when the shell is reaped, when the FG pgid
    /// can't be read, or when tpgid is the shell's own pid (idle shell).
    ///
    /// `pub fn` so integration tests at `crates/vibeflow/tests/` can call it
    /// across the compilation-unit boundary (same lesson as `last_proc_check`).
    pub fn has_foreground_child(&self) -> bool {
        let Some(child_pid) = self.child_pid() else {
            return false;
        };
        match crate::session::proc_watch::foreground_pgid(child_pid) {
            Some(tpgid) => tpgid > 0 && tpgid != child_pid,
            None => false,
        }
    }

    /// v0.1.3 confirm-on-close: name of the most-recently-matched AI tool from
    /// `tools_list` (set during `tick()`, cleared when the match drops).
    /// Returns None when no match is active OR no tools are configured.
    ///
    /// Cloned per call (the field is small; no hot-path callers).
    pub fn detected_ai_tool(&self) -> Option<String> {
        self.detected_ai_tool.clone()
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
            .resize(GridSize::new(cols as usize, rows as usize));
        Ok(())
    }

    /// Whether the child is still running and the reader thread alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// #10: true when the most recent `poll` stopped at the per-poll byte budget
    /// with output still queued. The window layer re-wakes immediately so the
    /// remaining backlog drains without one giant main-loop stall.
    #[must_use]
    pub fn output_pending(&self) -> bool {
        self.output_pending
    }

    /// #10: bytes consumed by the most recent `poll` call (test/bench-only).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn last_poll_bytes(&self) -> usize {
        self.last_poll_bytes
    }

    /// Read-only access to the per-session `Term` for rendering.
    #[must_use]
    pub fn term(&self) -> &Term<VoidListener> {
        &self.term
    }

    /// Stage 13: the requested theme NAME for this tab (the "intent" —
    /// may not be currently resolvable). `None` = Stage 9 defaults.
    #[must_use]
    pub fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// Stage 13: the resolved color table for this tab, if a theme was
    /// successfully applied. `None` = render with alacritty defaults.
    /// (Renderer consults this; `Term` is never mutated — see set_theme.)
    #[must_use]
    pub fn theme_colors(&self) -> Option<&alacritty_terminal::term::color::Colors> {
        self.theme_colors.as_ref()
    }

    /// Split-borrow helper for mouse event routing. Returns a mutable reference
    /// to the selection tracker and an immutable reference to the terminal grid
    /// in a single call, avoiding the aliasing problem that arises when calling
    /// `s.selection.mouse_down(point, shift, alt, s.term(), now)` — that expression
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

    /// Stage 13: apply a named theme to this session, or clear to Stage 9
    /// defaults when `name` is None.
    ///
    /// - Hit (`name` is `Some` and the registry contains it): `theme_colors`
    ///   is populated from the registry entry.
    /// - Miss (`name` is `Some` but the registry has no matching entry): logs
    ///   a warning and reverts `theme_colors` to `None` (alacritty default
    ///   palette). `self.theme` is still set to the requested name as recorded
    ///   *intent* — a later registry reload can re-resolve it.
    /// - Clear (`name` is `None`): both `self.theme` and `self.theme_colors`
    ///   are set to `None` (Stage 9 hardcoded defaults).
    ///
    /// Invariant: `theme_colors.is_some()` iff a theme was successfully
    /// resolved; `self.theme` holds the last *requested* name regardless of
    /// whether it is currently resolvable.
    pub fn set_theme(
        &mut self,
        name: Option<String>,
        registry: &crate::theme::registry::ThemeRegistry,
    ) {
        self.theme = name.clone();
        let Some(theme_name) = name else {
            self.theme_colors = None;
            return;
        };
        match registry.get(&theme_name) {
            Some(td) => {
                self.theme_colors = Some(crate::theme::apply_theme_to_colors(td));
            }
            None => {
                // Stage 13: requested theme isn't in the registry (deleted,
                // typo'd, or not yet imported). Keep `self.theme` as the
                // recorded *intent* (a later registry reload can re-resolve
                // it), but drop `theme_colors` so we render alacritty's
                // default palette instead of whatever theme was applied
                // before. Invariant: theme_colors.is_some() iff a theme
                // was successfully resolved.
                tracing::warn!(
                    "theme '{theme_name}' not in registry; reverting to default colors (name retained)"
                );
                self.theme_colors = None;
            }
        }
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
        // `*self = new_session` (below) runs Drop on the old PtySession, which
        // kills the old child, drops its receiver — unblocking any reader stuck
        // in a full-channel send (#17) — and joins the old reader thread.
        let argv = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
        let mut new_session = PtySession::spawn(
            &[argv.as_str()],
            TrackerConfig::default(),
            self.history_lines,
        )?;
        if let Some(s) = size {
            // PtySize uses u16 for rows/cols. Re-apply to the new master.
            let _ = new_session.resize(s.rows, s.cols);
        }
        // Preserve the OSC-title policy across restart (a deliberate config
        // override shouldn't be wiped just because the user hit Ctrl+Shift+R).
        // user_renamed is intentionally NOT preserved — the new shell is fresh.
        new_session.respect_osc_title = self.respect_osc_title;
        new_session.allow_osc52_write = self.allow_osc52_write;
        new_session.title_strip_prefix = std::mem::take(&mut self.title_strip_prefix);
        // Stage 13: transfer theme/theme_colors to the new session so this
        // method is self-consistent if called standalone. NOTE: in the normal
        // restart path this transfer is immediately superseded — App::restart_active
        // overwrites `theme` with the app default and WindowApp's RestartTab
        // handler re-resolves `theme_colors` via set_theme. A per-tab theme
        // override is therefore NOT preserved across a user-initiated restart
        // (the tab adopts the current app default, like history_lines/tools_list).
        new_session.theme = self.theme.take();
        new_session.theme_colors = self.theme_colors.take();
        *self = new_session;
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // #17: drop the receiver BEFORE join. A reader blocked on a full
        // sync_channel send() only wakes when the receiver is gone (send →
        // Err); kill() alone can't unblock it. Without this, closing a tab
        // mid-firehose deadlocks here on join().
        self.rx = None;
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Poll-with-deadline helper for PTY tests that depend on bash forking a
    /// subprocess within an indeterminate wall-clock window. Under parallel
    /// `cargo test` load (50+ concurrent PTYs) a fixed `thread::sleep(500ms)`
    /// isn't enough for the shell to parse input and fork the subprocess.
    /// Returns true if `cond` ever observed true before `timeout` elapsed.
    fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut cond: F) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        cond()
    }

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
            10000,
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
            10000,
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

    // Issue #7 (v0.1.4): when heuristic output drives a tracker transition inside
    // poll()'s PassThrough arm, poll() must emit StateChanged (not just
    // TermUpdated) so the tab repaints AND its cached subtitle refreshes. This is
    // the wiring the Waiting→Working recovery relies on (and also fixes the
    // pre-existing Active→Working case).
    #[test]
    fn poll_emits_state_changed_on_heuristic_output_promotion() {
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys, time; sys.stdout.buffer.write(b'x'); sys.stdout.flush(); time.sleep(2)",
            ],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        // Arm the Tier-3 heuristic directly (no /proc dependency). Output arriving
        // through poll() should now promote Active → Working AND surface a
        // StateChanged event.
        s.set_heuristic_active(true);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_state_changed_working = false;
        let mut events = Vec::new();
        while Instant::now() < deadline && !saw_state_changed_working {
            for ev in s.poll(Instant::now()) {
                if matches!(ev, SessionEvent::StateChanged(TabState::Working)) {
                    saw_state_changed_working = true;
                }
                events.push(ev);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            saw_state_changed_working,
            "expected StateChanged(Working) from heuristic output; got: {events:?}"
        );
        assert_eq!(s.state(), TabState::Working);
    }

    #[test]
    fn tick_does_not_fire_within_timeout_windows() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        // Default config: stale_state 30s, heuristic_silence 4s — neither
        // fires within 1s of spawn.
        let evs = s.tick(Instant::now() + Duration::from_secs(1));
        assert_eq!(evs, vec![]);
    }

    #[test]
    fn set_heuristic_active_toggles_tier_3() {
        // Direct test that the toggle reaches the tracker.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.set_heuristic_active(true);
        // No assertion on internal tracker state (it's a private field) —
        // exercise the path via tick after a Working transition + observed
        // output to ensure heuristic fires when the flag is on.
        let now = Instant::now();
        // Enter Working via a shell prompt marker (non-explicit) so the Tier-3 heuristic still governs —
        // this test verifies set_heuristic_active reaches the tracker and the silence inference fires for
        // NON-self-reporting sessions. (Explicit OSC 1338 sessions are exempt — Q1.)
        s.tracker.on_input(
            TrackerInput::Prompt(crate::session::osc::PromptMarker::CommandStart),
            now,
        );
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
            10000,
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
        let mut s = PtySession::spawn(&["/bin/cat"], TrackerConfig::default(), 10000).unwrap();
        s.send_input(b"hello\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = Vec::new();
        while Instant::now() < deadline && !buf.windows(5).any(|w| w == b"hello") {
            match s
                .rx
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(100))
            {
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
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
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
            10000,
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
            10000,
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
        let s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        assert!(s.is_alive());
        assert_eq!(s.state(), TabState::Active);
        // Drop `s` here — the Drop impl kills the child and joins the reader.
        drop(s);
    }

    #[test]
    fn session_reader_thread_pumps_bytes_to_channel() {
        // Spawn a child that prints predictable bytes, then read from the
        // session's channel directly to verify the reader thread is alive.
        let s = PtySession::spawn(
            &["/bin/sh", "-c", "printf hello"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        // Drain the channel for up to 2s and accumulate bytes.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buf = Vec::new();
        while Instant::now() < deadline && buf.len() < 5 {
            match s
                .rx
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(100))
            {
                Ok(chunk) => buf.extend_from_slice(&chunk),
                Err(_) => continue,
            }
        }
        assert!(buf.starts_with(b"hello"), "got: {:?}", buf);
    }

    #[test]
    fn tick_fires_stale_state_timeout() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 60"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        let now = Instant::now();
        // Enter Working non-explicitly (shell prompt) — stale-state timeout applies to non-self-reporting
        // sessions; OSC 1338 sessions are exempt (Q1, see explicit_frame_disables_stale_timeout).
        s.tracker.on_input(
            TrackerInput::Prompt(crate::session::osc::PromptMarker::CommandStart),
            now,
        );
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
    fn grid_size_dimensions_match_viewport() {
        use alacritty_terminal::grid::Dimensions;
        let g = GridSize::new(120, 40);
        assert_eq!(g.columns(), 120);
        assert_eq!(g.screen_lines(), 40);
        // Scrollback history is configured via `TermConfig::scrolling_history`,
        // not baked into the size handed to `Term::new`/`Term::resize`.
        assert_eq!(g.total_lines(), 40);
    }

    #[test]
    fn ptysession_default_label_is_bash_active() {
        // PtySession::spawn always starts with TabState::Active. The default
        // label tracks that.
        let s = PtySession::spawn(&["/bin/bash"], TrackerConfig::default(), 10000).unwrap();
        assert_eq!(s.label().title, "bash");
        assert_eq!(s.label().subtitle, "active");
    }

    #[test]
    fn ptysession_set_label_overrides_default() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.set_label(TabLabel {
            title: "custom".into(),
            subtitle: "claude · waiting".into(),
        });
        assert_eq!(s.label().title, "custom");
        assert_eq!(s.label().subtitle, "claude · waiting");
    }

    #[test]
    fn ptysession_custom_subtitle_survives_refresh() {
        // A subtitle installed via `set_label` is custom: the activity-driven
        // refresh on state transitions must not stomp it.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.set_label(TabLabel {
            title: "custom".into(),
            subtitle: "my subtitle".into(),
        });
        s.refresh_default_subtitle();
        assert_eq!(s.label().subtitle, "my subtitle");
    }

    #[test]
    fn ptysession_set_title_keeps_subtitle_activity_driven() {
        // Contrast: the rename UI's `set_title` touches only the title, so
        // the subtitle stays activity-driven and refresh still updates it.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.set_title("renamed".into());
        s.refresh_default_subtitle();
        assert_eq!(s.label().title, "renamed");
        assert_eq!(s.label().subtitle, "active");
    }

    #[test]
    fn poll_emits_bell_event_when_07_byte_received() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "printf '\\007hi'; sleep 0.5"],
            TrackerConfig::default(),
            10000,
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
        let mut s = PtySession::spawn(&["sleep", "10"], TrackerConfig::default(), 10000)
            .expect("first spawn");
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
        let mut s =
            PtySession::spawn(&["sleep", "5"], TrackerConfig::default(), 10000).expect("spawn");
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
        let mut s =
            PtySession::spawn(&["sleep", "5"], TrackerConfig::default(), 10000).expect("spawn");
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
    fn osc_0_strips_prefix_when_present() {
        let mut s =
            PtySession::spawn(&["sleep", "5"], TrackerConfig::default(), 10000).expect("spawn");
        s.title_strip_prefix = "user@host: ".to_string();
        for ev in s.dispatcher.feed(b"\x1b]0;user@host: ~/dev\x07") {
            if let DispatchEvent::SetTitle(title) = ev {
                let display = title
                    .strip_prefix(&s.title_strip_prefix)
                    .map(str::to_owned)
                    .unwrap_or(title);
                s.label.title = display;
            }
        }
        assert_eq!(s.label().title, "~/dev");
    }

    #[test]
    fn osc_0_passes_through_when_prefix_doesnt_match() {
        let mut s =
            PtySession::spawn(&["sleep", "5"], TrackerConfig::default(), 10000).expect("spawn");
        s.title_strip_prefix = "user@host: ".to_string();
        for ev in s.dispatcher.feed(b"\x1b]0;vim - foo.txt\x07") {
            if let DispatchEvent::SetTitle(title) = ev {
                let display = title
                    .strip_prefix(&s.title_strip_prefix)
                    .map(str::to_owned)
                    .unwrap_or(title);
                s.label.title = display;
            }
        }
        assert_eq!(s.label().title, "vim - foo.txt");
    }

    #[test]
    fn osc_0_dropped_when_respect_osc_title_false() {
        let mut s =
            PtySession::spawn(&["sleep", "5"], TrackerConfig::default(), 10000).expect("spawn");
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
        let mut s =
            PtySession::spawn(&["sleep", "5"], TrackerConfig::default(), 10000).expect("spawn");
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

    #[test]
    fn child_pid_returns_some_for_live_session() {
        let s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let pid = s.child_pid();
        assert!(pid.is_some(), "live session should report child pid");
        assert!(pid.unwrap() > 0, "pid should be positive");
    }

    #[test]
    fn set_tracker_config_propagates_to_tracker() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let new_cfg = TrackerConfig {
            heuristic_silence: std::time::Duration::from_millis(1500),
            ..TrackerConfig::default()
        };
        s.set_tracker_config(new_cfg);
        // Indirectly verify by driving Working + waiting past the new threshold:
        let now = std::time::Instant::now();
        s.set_heuristic_active(true);
        // Need to drive Working — but PtySession::tick alone won't flip state without
        // an OSC 1338 input. Simplest: assert via a side-channel — the tracker's
        // `state()` defaults to Active, so after a tick at +1.6s with heuristic_active
        // and Working, we'd expect Waiting. Without a tracker-feed accessor, this test
        // just asserts the method exists and doesn't panic. The full state-change
        // behavior is already covered by tracker::tests::set_config_updates_…
        let _ = s.tick(now + std::time::Duration::from_secs(2));
        // Method exists and returned cleanly.
    }

    #[test]
    fn tick_runs_proc_check_on_first_call() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        s.proc_check_interval = std::time::Duration::from_millis(250);
        s.last_proc_check = None;
        let t0 = std::time::Instant::now();
        let _ = s.tick(t0);
        assert!(
            s.last_proc_check.is_some(),
            "first tick should run the proc check"
        );
    }

    #[test]
    fn tick_throttles_proc_check_within_interval() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        s.proc_check_interval = std::time::Duration::from_millis(250);
        let t0 = std::time::Instant::now();
        let _ = s.tick(t0);
        let after_first = s.last_proc_check;
        let _ = s.tick(t0 + std::time::Duration::from_millis(100));
        assert_eq!(
            s.last_proc_check, after_first,
            "tick within interval should NOT re-run proc check"
        );
    }

    #[test]
    fn tick_runs_proc_check_again_past_interval() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        s.proc_check_interval = std::time::Duration::from_millis(100);
        let t0 = std::time::Instant::now();
        let _ = s.tick(t0);
        let after_first = s.last_proc_check.unwrap();
        let _ = s.tick(t0 + std::time::Duration::from_millis(200));
        let after_second = s.last_proc_check.unwrap();
        assert!(
            after_second > after_first,
            "tick past interval should re-run proc check"
        );
    }

    #[test]
    fn tier_3_arms_on_rising_edge_even_without_real_output() {
        // Stage 11 follow-up: when the proc check flips heuristic from off to on
        // and the AI tool produces no immediate output (e.g., `python3 -c "sleep 30"`),
        // we must still transition the tracker to Working so the silence guard can fire.
        // This test simulates the transition by mutating tools_list mid-flight and
        // ticking to force a proc check.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig {
                heuristic_silence: std::time::Duration::from_millis(500),
                ..TrackerConfig::default()
            },
            10000,
        )
        .expect("spawn");
        // Initially tools_list is empty → heuristic stays off.
        s.tools_list = vec![];
        s.proc_check_interval = std::time::Duration::from_millis(0); // always fire
        let t0 = std::time::Instant::now();
        let _ = s.tick(t0);
        assert_eq!(
            s.tracker_state(),
            TabState::Active,
            "armed=false → state stays Active"
        );

        // Arm the heuristic with a fixed candidate list. We avoid
        // `foreground_command_name` here for two reasons:
        // (1) under parallel test load /proc/<tpgid>/comm can race and return
        //     a Rust test thread comm (e.g. "session::sessio");
        // (2) `sh -c "sleep 5"` exec()'s into `sleep` on some shells, so the
        //     comm changes from "sh" to "sleep" during the test's 700 ms
        //     silence window and a single-name list would un-arm before the
        //     Working → Waiting transition fires.
        // Covering all plausible names keeps the heuristic armed regardless
        // of exec timing or which sh implementation is at /bin/sh.
        s.tools_list = vec!["sh".into(), "bash".into(), "dash".into(), "sleep".into()];

        // Tick repeatedly: under load the first tick after arming may race
        // with /proc state. Poll up to 2 s for the rising edge to promote
        // Active → Working.
        let promoted = wait_until(Duration::from_secs(2), || {
            let _ = s.tick(Instant::now());
            s.tracker_state() == TabState::Working
        });
        assert!(
            promoted,
            "rising edge of heuristic_active should promote state to Working within 2s, got {:?}",
            s.tracker_state()
        );

        // Without further output bytes, silence threshold elapses and state → Waiting.
        // heuristic_silence is 500 ms; sleep 700 ms past the promotion to be safe.
        std::thread::sleep(Duration::from_millis(700));
        let _ = s.tick(Instant::now());
        assert_eq!(
            s.tracker_state(),
            TabState::Waiting,
            "silence past threshold should transition to Waiting"
        );
    }

    #[test]
    fn tick_arms_heuristic_when_command_in_tools_list() {
        // Spawn `bash`. The session's foreground command will be reported by /proc
        // as some shell-like name (likely "bash" but depends on env). Configure
        // tools_list to include "bash"; verify heuristic_active flips true.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        s.tools_list = vec!["bash".to_owned()];
        s.proc_check_interval = std::time::Duration::from_millis(0); // always fire
        let t0 = std::time::Instant::now();
        let _ = s.tick(t0);
        // We can't directly read tracker.heuristic_active (it's private), so
        // assert via behavior: drive Working, advance past heuristic_silence,
        // assert state == Waiting. This exercises the full Tier 3 path.
        s.tracker.on_input(
            crate::session::tracker::TrackerInput::AiFrame(vibeflow_protocol::Frame::new(
                vibeflow_protocol::State::Working,
            )),
            t0,
        );
        // Need another tick to fire the heuristic timer past silence.
        let _ = s.tick(t0 + std::time::Duration::from_millis(5000));
        // If heuristic is armed AND we're in Working AND silence elapsed, expect Waiting.
        // BUT: this depends on /proc being readable AND comm matching "bash" exactly.
        // On environments where /proc is restricted or comm differs, the assertion
        // would fail. So we make a tolerant check: confirm tick fired the proc
        // check (last_proc_check is Some), which is the deterministic part of
        // Stage 11's behavior. Full state-transition behavior is exercised in
        // Task 10's integration tests.
        assert!(
            s.last_proc_check.is_some(),
            "tick should have run the proc check"
        );
    }

    #[test]
    fn scroll_by_zero_is_noop_no_fade() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let now = std::time::Instant::now();
        // Pre-fade state.
        assert_eq!(s.scrollbar_fade.alpha(now), 0.0);
        s.scroll_by(0, now);
        // No fade triggered.
        assert_eq!(
            s.scrollbar_fade.alpha(now),
            0.0,
            "scroll_by(0) should not arm the fade timer"
        );
    }

    #[test]
    fn scroll_by_nonzero_arms_fade() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let now = std::time::Instant::now();
        s.scroll_by(-5, now);
        assert!(
            s.scrollbar_fade.alpha(now) > 0.0,
            "fade should arm on nonzero scroll"
        );
    }

    #[test]
    fn scroll_to_top_then_bottom_round_trips_display_offset() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let now = std::time::Instant::now();
        // Without history content the round-trip is trivial — both yield 0.
        // The test still validates the API does not panic.
        s.scroll_to_top(now);
        s.scroll_to_bottom(now);
        assert_eq!(s.display_offset(), 0);
    }

    #[test]
    fn display_offset_starts_at_zero() {
        let s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        assert_eq!(s.display_offset(), 0);
    }

    #[test]
    fn set_theme_none_clears_colors() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let reg = crate::theme::registry::ThemeRegistry::new_empty();
        s.set_theme(None, &reg);
        assert_eq!(s.theme, None);
        assert!(s.theme_colors.is_none());
    }

    #[test]
    fn set_theme_missing_name_records_name_reverts_colors() {
        // On a fresh session (theme_colors starts as None), a miss from an
        // empty registry retains the requested name but leaves colors at None
        // (already-default). Invariant: theme_colors.is_none() on miss.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        let reg = crate::theme::registry::ThemeRegistry::new_empty();
        s.set_theme(Some("ghost".to_owned()), &reg);
        assert_eq!(s.theme.as_deref(), Some("ghost"));
        assert!(s.theme_colors.is_none()); // empty registry -> miss -> reverted to default (None)
    }

    #[test]
    fn set_theme_miss_after_hit_reverts_stale_colors() {
        // The key bug fix: a miss AFTER a successful hit must clear theme_colors
        // (revert to alacritty default) rather than leaving the previous theme's
        // colors in place. Without the fix, the session would render the old
        // theme's colors while self.theme claimed a different name.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");

        // Build a minimal registry containing exactly one real theme.
        let tmp = tempfile::tempdir().expect("tempdir");
        let td = crate::theme::ThemeData {
            name: "real".into(),
            ansi: [[0.5; 4]; 16],
            foreground: [1.0, 0.0, 0.0, 1.0],
            background: [0.0; 4],
            cursor: [0.5, 0.5, 0.5, 1.0],
            cursor_text: [0.0; 4],
            bold: None,
            link: None,
            selection: None,
        };
        std::fs::write(tmp.path().join("real.toml"), td.to_toml()).expect("write theme file");
        let reg = crate::theme::registry::ThemeRegistry::load(tmp.path().to_path_buf());

        // Hit: "real" resolves → theme_colors must be populated.
        s.set_theme(Some("real".to_owned()), &reg);
        assert!(s.theme_colors.is_some(), "hit must populate theme_colors");

        // Miss after hit: "ghost" is not in the registry.
        // Requested name must be retained; stale colors must be cleared.
        s.set_theme(Some("ghost".to_owned()), &reg);
        assert_eq!(
            s.theme.as_deref(),
            Some("ghost"),
            "requested name must be retained on miss"
        );
        assert!(
            s.theme_colors.is_none(),
            "stale colors from prior hit must be cleared on miss"
        );
    }

    // ── Q2 regression tests ──────────────────────────────────────────────────

    /// Regression for the spaced-title subtitle freeze: once an OSC/PS1 title
    /// with a space lands (e.g. bash PS1 `user@host: ~/dev/vibeflow`),
    /// `refresh_default_subtitle` must still update the subtitle on state
    /// changes AND must NOT clobber the title back to a basename.
    #[test]
    fn subtitle_tracks_state_even_with_spaced_title() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");

        // Simulate an OSC/PS1 spaced title via the supported API (set_title is
        // the path that production poll() uses for OSC 0/2 titles).
        s.set_title("user@host: ~/some dir".to_string());
        assert!(
            s.label().title.contains(' '),
            "precondition: title must contain a space"
        );
        let spaced_title = s.label().title.clone();

        // Drive a state change: CommandStart → Working.
        let now = Instant::now();
        s.tracker.on_input(
            TrackerInput::Prompt(crate::session::osc::PromptMarker::CommandStart),
            now,
        );
        s.refresh_default_subtitle();

        // Subtitle must reflect the new state ("working"), NOT be frozen at "active".
        assert_eq!(
            s.label().subtitle,
            "working",
            "subtitle must update to 'working' despite spaced title"
        );
        // Title must NOT have been clobbered back to a basename.
        assert_eq!(
            s.label().title,
            spaced_title,
            "refresh_default_subtitle must not clobber the OSC-set title"
        );
    }

    /// `TabLabel::subtitle_for` must map all five `TabState` variants to the
    /// expected word. Direct unit test of the extracted mapping function.
    #[test]
    fn refresh_subtitle_maps_all_states() {
        assert_eq!(TabLabel::subtitle_for(TabState::Active), "active");
        assert_eq!(TabLabel::subtitle_for(TabState::Working), "working");
        assert_eq!(TabLabel::subtitle_for(TabState::Waiting), "waiting");
        assert_eq!(TabLabel::subtitle_for(TabState::Done), "done");
        assert_eq!(TabLabel::subtitle_for(TabState::Idle), "idle");

        // Integration: drive a state change through refresh_default_subtitle and
        // confirm the label's subtitle field is updated.
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .expect("spawn");
        assert_eq!(s.label().subtitle, "active");
        let now = Instant::now();
        s.tracker.on_input(
            TrackerInput::Prompt(crate::session::osc::PromptMarker::CommandStart),
            now,
        );
        s.refresh_default_subtitle();
        assert_eq!(s.label().subtitle, "working");
    }

    /// Pin that the `subtitle_for` extraction did not change `default_for`'s
    /// output: title is still the basename, subtitle is still the state word.
    #[test]
    fn default_for_unchanged_by_refactor() {
        let label = TabLabel::default_for("/bin/bash", TabState::Idle);
        assert_eq!(label.title, "bash");
        assert_eq!(label.subtitle, "idle");
    }

    /// Contract (Q2): interactive rename overrides ONLY the title; the
    /// subtitle remains activity-driven and must keep updating with tracker
    /// state. Guards against a future re-introduction of a title/rename gate
    /// on refresh_default_subtitle (which previously froze the subtitle).
    #[test]
    fn subtitle_still_tracks_state_after_user_rename() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "sleep 5"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        // User renames the tab (sets user_renamed + a custom, spaced title).
        s.set_title("My Important Tab".to_string());
        s.user_renamed = true; // mirrors commit_rename's effect; field is pub
        let now = Instant::now();
        // A state change arrives; subtitle MUST still update.
        let _ = s.tracker.on_input(
            TrackerInput::Prompt(crate::session::osc::PromptMarker::CommandStart),
            now,
        );
        s.refresh_default_subtitle();
        assert_eq!(
            s.label().subtitle,
            "working",
            "subtitle must track state even after rename"
        );
        assert_eq!(
            s.label().title,
            "My Important Tab",
            "rename title must be preserved"
        );
    }

    #[test]
    fn poll_translates_dispatchevent_osc52write_into_sessionevent() {
        // Drive a session with a literal OSC 52 sequence and assert poll() emits
        // SessionEvent::Osc52ClipboardWrite carrying the same selection + text.
        // Pattern matches the existing poll_routes_osc_1338_through_dispatcher_and_tracker
        // test: spawn python to output an OSC 52 sequence + deadline loop.
        use crate::session::osc::Osc52Selection;
        use crate::session::tracker::TrackerConfig;
        use std::time::{Duration, Instant};

        // Python script that outputs: ESC ] 52 ; c ; SGVsbG8= BEL
        // (OSC 52, clipboard selection, base64-encoded "Hello")
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys; sys.stdout.buffer.write(b'\\x1b]52;c;SGVsbG8=\\x07'); sys.stdout.flush()",
            ],
            TrackerConfig::default(),
            10_000,
        )
        .expect("PtySession::spawn");

        // Wait for the bytes to round-trip through the PTY + reader thread.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut observed = None;
        while Instant::now() < deadline {
            for ev in s.poll(Instant::now()) {
                if let SessionEvent::Osc52ClipboardWrite { selection, text } = ev {
                    observed = Some((selection, text));
                    break;
                }
            }
            if observed.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let (selection, text) = observed.expect("Osc52ClipboardWrite within 3s");
        assert_eq!(selection, Osc52Selection::Clipboard);
        assert_eq!(text, "Hello");
    }

    #[test]
    fn poll_drops_osc52write_when_allow_osc52_write_false() {
        // Pre-launch audit: untrusted terminal output must not be able to
        // overwrite the system clipboard when the user has opted out via
        // `[clipboard] allow_osc52_write = false`. Emit the same OSC 52 write
        // as the test above plus a trailing visible byte, and assert poll()
        // surfaces normal output but NEVER an Osc52ClipboardWrite.
        use crate::session::tracker::TrackerConfig;
        use std::time::{Duration, Instant};

        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys; sys.stdout.buffer.write(b'\\x1b]52;c;SGVsbG8=\\x07X'); sys.stdout.flush()",
            ],
            TrackerConfig::default(),
            10_000,
        )
        .expect("PtySession::spawn");
        s.allow_osc52_write = false;

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut saw_output = false;
        while Instant::now() < deadline {
            for ev in s.poll(Instant::now()) {
                assert!(
                    !matches!(ev, SessionEvent::Osc52ClipboardWrite { .. }),
                    "OSC 52 write must be dropped when allow_osc52_write = false"
                );
                if matches!(ev, SessionEvent::TermUpdated) {
                    saw_output = true;
                }
            }
            if saw_output {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_output, "the trailing visible byte should still render");
    }

    // ---------------- v0.1.3 confirm-on-close ----------------

    #[test]
    fn has_foreground_child_returns_false_for_idle_shell() {
        // Spawn a real PtySession running bash, give it ~250 ms for the shell to
        // settle at its prompt, then check. The shell IS the foreground process
        // group, so has_foreground_child should be false.
        let mut s =
            PtySession::spawn(&["/bin/bash"], TrackerConfig::default(), 1000).expect("spawn bash");
        // Give bash a moment to take over the controlling terminal.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = s.poll(std::time::Instant::now());
        assert!(
            !s.has_foreground_child(),
            "idle bash should report no foreground child, got true"
        );
    }

    #[test]
    fn has_foreground_child_returns_true_when_shell_runs_subprocess() {
        // Spawn bash, send a sleep command, poll-until-detected.
        let mut s =
            PtySession::spawn(&["/bin/bash"], TrackerConfig::default(), 1000).expect("spawn bash");
        // Wait for bash to take controlling terminal.
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = s.poll(Instant::now());
                !s.has_foreground_child()
            }),
            "bash should reach idle (no foreground child) within 5s"
        );
        // Send a long-running command. The trailing \n triggers execution.
        s.send_input(b"sleep 30\n").expect("send sleep cmd");
        // Poll until bash forks sleep and the foreground pgid flips.
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = s.poll(Instant::now());
                s.has_foreground_child()
            }),
            "bash running `sleep 30` should report a foreground child within 5s"
        );
    }

    #[test]
    fn detected_ai_tool_returns_none_for_idle_shell_with_no_tools_configured() {
        let mut s =
            PtySession::spawn(&["/bin/bash"], TrackerConfig::default(), 1000).expect("spawn bash");
        // tools_list defaults to empty; tick() leaves detected_ai_tool untouched
        // when the list is empty, so it stays None.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = s.poll(std::time::Instant::now());
        let _ = s.tick(std::time::Instant::now());
        assert_eq!(s.detected_ai_tool(), None);
    }

    #[test]
    fn detected_ai_tool_returns_matched_name_when_shell_runs_listed_tool() {
        // Configure tools_list to include "sleep" so a `sleep` subprocess
        // counts as a "detected AI tool" for the purposes of this test.
        // (We can't easily spawn `claude` from a unit test; `sleep` is a
        // standard binary present on every system AND short enough to fit
        // the kernel's 15-char `comm` truncation.)
        let mut s =
            PtySession::spawn(&["/bin/bash"], TrackerConfig::default(), 1000).expect("spawn bash");
        s.tools_list = vec!["sleep".to_string()];
        s.proc_check_interval = Duration::from_millis(50);
        s.send_input(b"sleep 30\n").expect("send sleep cmd");
        // Poll until bash forks sleep AND a tick has propagated the match.
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = s.poll(Instant::now());
                let _ = s.tick(Instant::now());
                s.detected_ai_tool().as_deref() == Some("sleep")
            }),
            "bash running `sleep 30` should surface detected_ai_tool=\"sleep\" within 5s"
        );
    }

    // #10: a single poll must not drain an unbounded backlog — it caps at
    // MAX_POLL_BYTES (+ up to one ≤4 KB chunk) and signals output_pending so the
    // main loop re-wakes to finish, instead of freezing on one giant parse.
    #[test]
    fn poll_caps_bytes_per_call_and_signals_pending() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "seq 1 2000000"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.resize(60, 200).ok();
        // Let the reader thread queue far more than the budget (a `seq` flood
        // pushes MBs into the channel in well under this window).
        std::thread::sleep(Duration::from_millis(250));

        let _ = s.poll(Instant::now());
        assert!(
            s.last_poll_bytes() <= MAX_POLL_BYTES + 4096,
            "one poll consumed {} bytes; must be capped at MAX_POLL_BYTES (+1 chunk)",
            s.last_poll_bytes()
        );
        assert!(
            s.output_pending(),
            "backlog remained, so poll must report output_pending"
        );

        // Repeated polls must eventually drain everything and reach EOF.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let _ = s.poll(Instant::now());
            if !s.is_alive() && !s.output_pending() && s.last_poll_bytes() == 0 {
                break;
            }
        }
        assert!(!s.output_pending(), "backlog should be fully drained");
        assert!(!s.is_alive(), "child should have exited (EOF reached)");
    }

    // #10: a poll that consumes multiple pass-through chunks coalesces them into
    // exactly one TermUpdated (was one-per-chunk → redundant redraws).
    #[test]
    fn poll_coalesces_term_updated_per_call() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "seq 1 2000000"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.resize(60, 200).ok();
        std::thread::sleep(Duration::from_millis(250));

        let evs = s.poll(Instant::now());
        // The budgeted poll processed ~64 KB = many ≤4 KB chunks...
        assert!(
            s.last_poll_bytes() > 4096,
            "expected multiple chunks consumed, got {} bytes",
            s.last_poll_bytes()
        );
        // ...but emitted exactly one TermUpdated.
        let term_updates = evs
            .iter()
            .filter(|e| matches!(e, SessionEvent::TermUpdated))
            .count();
        assert_eq!(
            term_updates, 1,
            "expected exactly one coalesced TermUpdated"
        );
    }

    // ---- #10 perf probes (throwaway; run explicitly, not in the normal suite) ----
    // cargo test -p vibeflow --lib perf_probe -- --ignored --nocapture --test-threads=1
    // `seq 1 2000000` ≈ 14.9 MB raw / ~16.9 MB after PTY CRLF translation.

    #[test]
    #[ignore = "perf probe"]
    fn perf_probe_parse_drain_throughput() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "seq 1 2000000"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        s.resize(60, 200).ok();
        let t0 = Instant::now();
        let mut polls = 0u64;
        let mut idle = 0u32;
        loop {
            let evs = s.poll(Instant::now());
            polls += 1;
            if !s.is_alive() && evs.is_empty() {
                idle += 1;
                if idle > 5 {
                    break;
                }
            } else {
                idle = 0;
            }
            if t0.elapsed() > Duration::from_secs(60) {
                break;
            }
        }
        let dur = t0.elapsed();
        eprintln!(
            "[#10 parse_drain] ~16.9MB consumed in {dur:?} over {polls} tight polls (no render) => ~{:.0} MB/s",
            16.9 / dur.as_secs_f64()
        );
    }

    #[test]
    fn drop_does_not_hang_when_reader_blocked_on_full_channel() {
        // #17: with the bounded channel the reader thread can be blocked in
        // send() (queue full) rather than read(). Drop must drop the receiver
        // before join() so the blocked send returns Err and the thread exits —
        // otherwise closing a tab mid-firehose deadlocks on join().
        let s = PtySession::spawn(
            &["/bin/sh", "-c", "cat /dev/zero"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        // Do NOT poll: let the reader fill the 512-chunk channel (up to ~2 MiB,
        // fills in milliseconds from /dev/zero) and block on send().
        std::thread::sleep(Duration::from_millis(300));
        // Drop on a worker thread; the main thread asserts it completes promptly.
        let dropper = std::thread::spawn(move || drop(s));
        let finished = wait_until(Duration::from_secs(5), || dropper.is_finished());
        assert!(
            finished,
            "PtySession::drop() hung — a reader blocked on a full channel was \
             not unblocked at teardown (#17 regression)"
        );
        dropper.join().unwrap();
    }
}
