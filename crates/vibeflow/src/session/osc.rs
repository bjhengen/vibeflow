//! Streaming OSC dispatcher — recognises OSC 1338 (vibeflow's AI protocol) and
//! OSC 133 (shell-prompt integration), forwards everything else as pass-through
//! bytes for the terminal grid.

/// An OSC 133 "Terminal Integration" prompt marker.
///
/// Emitted when the dispatcher recognises one of the four standard subtypes.
/// Subtypes outside `A`/`B`/`C`/`D` are dropped silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMarker {
    /// `OSC 133;A` — start of prompt rendering.
    PromptStart,
    /// `OSC 133;B` — end of prompt; user can type now.
    PromptEnd,
    /// `OSC 133;C` — shell is about to run a command.
    CommandStart,
    /// `OSC 133;D[;<exit_code>]` — shell finished a command. `exit_code` is the
    /// command's status if the shell included it, otherwise `None`.
    CommandEnd { exit_code: Option<i32> },
}

/// Parse the body of an `OSC 133;…` sequence — the part *after* the `133;` prefix.
///
/// Returns `None` for unknown subtypes (caller drops the sequence). Garbage or
/// missing exit codes on `D` resolve to `CommandEnd { exit_code: None }`.
fn parse_133_body(body: &str) -> Option<PromptMarker> {
    let mut parts = body.split(';');
    let subtype = parts.next()?;
    match subtype {
        "A" => Some(PromptMarker::PromptStart),
        "B" => Some(PromptMarker::PromptEnd),
        "C" => Some(PromptMarker::CommandStart),
        "D" => {
            let exit_code = parts.next().and_then(|s| s.parse().ok());
            Some(PromptMarker::CommandEnd { exit_code })
        }
        _ => None,
    }
}

use vibeflow_protocol::Frame;

/// Maximum total length of a single OSC sequence (including `ESC ]` and the
/// terminator). Sequences exceeding this are dropped on the floor.
///
/// Sized for OSC 52 clipboard payloads: ~100 KB raw text becomes ~134 KB
/// after base64 encoding plus the `52;c;` prefix. 128 KB is a comfortable
/// envelope; other OSC types (0/2 title, 133 prompt markers, 1338 AI state)
/// are all under 1 KB in practice.
const MAX_OSC_LEN: usize = 131_072;

/// One event emitted by [`OscDispatcher::feed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchEvent {
    /// A complete OSC 1338 frame was parsed.
    AiState(Frame),
    /// An OSC 133 prompt marker was identified.
    Prompt(PromptMarker),
    /// OSC 0 (set window+icon title) or OSC 2 (set window title only).
    /// Carries the title payload as UTF-8. Stage 9.
    SetTitle(String),
    /// OSC 52 clipboard WRITE — TUI app asked us to replace the named
    /// clipboard selection(s) with `text`. Window layer dispatches to
    /// `Clipboard::copy_clipboard_only` / `copy_primary` based on `selection`.
    /// Read requests are silently dropped by the dispatcher (no event).
    Osc52Write {
        selection: Osc52Selection,
        text: String,
    },
    /// Bytes that should be forwarded to the terminal grid (alacritty_terminal in
    /// future stages). Includes any unknown OSC sequences (their original bytes,
    /// terminator and all) plus all non-OSC bytes.
    PassThrough(Vec<u8>),
}

/// Which clipboard selection(s) an OSC 52 write should target.
///
/// The OSC 52 selection field is a string of letters; `c` = system CLIPBOARD,
/// `p` = X11 PRIMARY selection, `s` = both (xterm convention). Other letters
/// (`q`, `0`..`7` for cut-buffers) are not supported and are filtered out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osc52Selection {
    Clipboard,
    Primary,
    Both,
}

/// Parse an OSC 52 selection field. `s` always means BOTH (xterm convention,
/// short-circuits on first occurrence). Otherwise the set of letters is
/// inspected: `c` selects CLIPBOARD, `p` selects PRIMARY, both together means
/// BOTH. Empty or unknown-only strings fall back to CLIPBOARD as a safe default.
fn parse_selection(s: &str) -> Osc52Selection {
    let mut has_c = false;
    let mut has_p = false;
    for ch in s.chars() {
        match ch {
            'c' => has_c = true,
            'p' => has_p = true,
            's' => return Osc52Selection::Both,
            _ => {}
        }
    }
    match (has_c, has_p) {
        (true, true) => Osc52Selection::Both,
        (false, true) => Osc52Selection::Primary,
        // (true, false) AND (false, false) both fall through to Clipboard:
        // explicit `c` is honoured; empty/unknown-only is the safe default.
        _ => Osc52Selection::Clipboard,
    }
}

/// Outcome of parsing an OSC 52 body.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Osc52ParseOutcome {
    /// Valid write request.
    Write {
        selection: Osc52Selection,
        text: String,
        /// True when the decoded payload exceeded `MAX_OSC52_RAW_BYTES` and
        /// was clipped. Caller logs a warn.
        truncated: bool,
    },
    /// Read request (`?` payload). Intentionally not implemented for security.
    ReadIgnored,
    /// Body did not match the expected `<selection>;<base64-or-?>` shape, OR
    /// base64 decoding failed.
    Malformed,
}

/// Cap on raw text from a single OSC 52 write. Larger payloads are clipped
/// to this length with the `truncated` flag set. The MAX_OSC_LEN envelope
/// covers ~3× this in base64 + overhead.
const MAX_OSC52_RAW_BYTES: usize = 100 * 1024;

/// Parse the body of an `OSC 52;...` sequence (the part AFTER `52;`).
fn parse_52_body(body: &str) -> Osc52ParseOutcome {
    let Some((sel_str, payload)) = body.split_once(';') else {
        return Osc52ParseOutcome::Malformed;
    };
    let selection = parse_selection(sel_str);

    if payload == "?" {
        return Osc52ParseOutcome::ReadIgnored;
    }

    use base64::Engine;
    let decoded = match base64::engine::general_purpose::STANDARD.decode(payload) {
        Ok(d) => d,
        Err(_) => return Osc52ParseOutcome::Malformed,
    };

    let (text_bytes, truncated) = if decoded.len() > MAX_OSC52_RAW_BYTES {
        (&decoded[..MAX_OSC52_RAW_BYTES], true)
    } else {
        (&decoded[..], false)
    };

    let text = String::from_utf8_lossy(text_bytes).into_owned();
    Osc52ParseOutcome::Write { selection, text, truncated }
}

/// Internal parser state. Tracks whether we're scanning plain bytes, have just
/// seen an `ESC`, are inside an OSC body buffering toward the terminator, or
/// have seen an `ESC` *inside* an OSC body (potential start of `ESC \` ST).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    Plain,
    SeenEsc,
    InOsc,
    InOscEsc, // ESC inside OSC; if next byte is `\`, terminate as ST
}

/// Streaming OSC dispatcher.
///
/// Feed bytes incrementally with [`OscDispatcher::feed`]; each call returns a
/// `Vec<DispatchEvent>` ordered by where each event falls in the input. Internal
/// state is preserved across calls so partial sequences split across reads are
/// handled correctly.
#[derive(Debug)]
pub struct OscDispatcher {
    state: ParseState,
    /// Bytes seen so far in the current OSC body (after `ESC ]`, before terminator).
    osc_body: Vec<u8>,
    /// Pending pass-through bytes accumulated since the last emitted event.
    /// Flushed at the end of each `feed` call (or when an OSC starts).
    pass_buf: Vec<u8>,
    /// True once the current OSC body has overflowed `MAX_OSC_LEN`. We keep
    /// scanning for the terminator but discard the body and emit nothing.
    osc_overflowed: bool,
}

impl OscDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: ParseState::Plain,
            osc_body: Vec::with_capacity(64),
            pass_buf: Vec::with_capacity(256),
            osc_overflowed: false,
        }
    }

    /// Feed a chunk of bytes into the dispatcher; returns events in input order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<DispatchEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            self.step(b, &mut events);
        }
        // Flush any pending pass-through at the end of the chunk.
        if !self.pass_buf.is_empty() {
            events.push(DispatchEvent::PassThrough(std::mem::take(
                &mut self.pass_buf,
            )));
        }
        events
    }

    /// Process a single byte. State transitions only.
    fn step(&mut self, b: u8, events: &mut Vec<DispatchEvent>) {
        match self.state {
            ParseState::Plain => {
                if b == 0x1B {
                    self.state = ParseState::SeenEsc;
                } else {
                    self.pass_buf.push(b);
                }
            }
            ParseState::SeenEsc => {
                if b == b']' {
                    // OSC introducer — flush pending passthrough first so events
                    // arrive in input order, then enter OSC body parsing.
                    self.flush_pass(events);
                    self.state = ParseState::InOsc;
                    self.osc_body.clear();
                    self.osc_overflowed = false;
                } else if b == 0x1B {
                    // Two ESCs in a row. The first ESC was a false start (this
                    // byte is ESC, not `]`). Emit the first ESC as plain and
                    // treat this second ESC as a fresh OSC-introducer candidate.
                    self.pass_buf.push(0x1B);
                    // state stays SeenEsc with this ESC pending
                } else {
                    // Not an OSC — restore ESC + this byte as plain bytes.
                    self.pass_buf.push(0x1B);
                    self.pass_buf.push(b);
                    self.state = ParseState::Plain;
                }
            }
            ParseState::InOsc => {
                if b == 0x07 {
                    // BEL terminator
                    self.finish_osc(events);
                } else if b == 0x1B {
                    // Could be the start of an `ESC \` ST terminator
                    self.state = ParseState::InOscEsc;
                } else {
                    self.push_osc_byte(b);
                }
            }
            ParseState::InOscEsc => {
                if b == b'\\' {
                    // ESC \ — ST terminator
                    self.finish_osc(events);
                } else {
                    // ESC inside an OSC body that didn't form ST — treat the
                    // ESC as starting a new OSC introducer attempt; drop the
                    // current OSC (we have no way to recover its terminator).
                    // This is the "malformed OSC" path. The current byte still
                    // needs to be processed: re-feed it from a fresh state.
                    self.osc_body.clear();
                    self.osc_overflowed = false;
                    self.state = ParseState::SeenEsc;
                    self.step(b, events);
                }
            }
        }
    }

    fn flush_pass(&mut self, events: &mut Vec<DispatchEvent>) {
        if !self.pass_buf.is_empty() {
            events.push(DispatchEvent::PassThrough(std::mem::take(
                &mut self.pass_buf,
            )));
        }
    }

    fn push_osc_byte(&mut self, b: u8) {
        if self.osc_overflowed {
            return;
        }
        // +2 accounts for the ESC ] header that's not in osc_body but counts
        // toward MAX_OSC_LEN; +1 for the terminator we'll see soon.
        if self.osc_body.len() + 3 >= MAX_OSC_LEN {
            self.osc_overflowed = true;
            return;
        }
        self.osc_body.push(b);
    }

    fn finish_osc(&mut self, events: &mut Vec<DispatchEvent>) {
        let body = std::mem::take(&mut self.osc_body);
        let overflowed = std::mem::replace(&mut self.osc_overflowed, false);
        // Track which terminator we saw — needed to reconstruct an unknown OSC.
        // self.state at this point is either InOsc (BEL termination) or
        // InOscEsc (ESC \ termination).
        let used_st = self.state == ParseState::InOscEsc;
        self.state = ParseState::Plain;

        if overflowed {
            return;
        }

        match handle_osc(&body) {
            OscOutcome::Event(ev) => events.push(ev),
            OscOutcome::Drop => {}
            OscOutcome::Forward => {
                // Reconstruct the original sequence and emit as PassThrough.
                let mut full = Vec::with_capacity(body.len() + 4);
                full.push(0x1B);
                full.push(b']');
                full.extend_from_slice(&body);
                if used_st {
                    full.push(0x1B);
                    full.push(b'\\');
                } else {
                    full.push(0x07);
                }
                events.push(DispatchEvent::PassThrough(full));
            }
        }
    }
}

/// What `handle_osc` decided to do with a complete OSC body.
enum OscOutcome {
    /// We recognised the OSC and produced this event.
    Event(DispatchEvent),
    /// We recognised the OSC ID but the body was malformed for it (e.g. an
    /// OSC 1338 with an unknown state value). Drop silently — log debug in
    /// future stages.
    Drop,
    /// We don't own this OSC ID. Caller should emit a PassThrough with the
    /// original bytes (ESC ] body terminator) intact.
    Forward,
}

fn handle_osc(body: &[u8]) -> OscOutcome {
    let Some(body_str) = std::str::from_utf8(body).ok() else {
        // Non-UTF-8 body. We don't own this OSC; let the terminal try.
        return OscOutcome::Forward;
    };
    let (id, params) = body_str.split_once(';').unwrap_or((body_str, ""));
    match id {
        "0" | "2" => {
            // OSC 0 sets both window + icon title; OSC 2 sets only window
            // title. We don't distinguish icon from title — both update
            // `TabLabel.title` via DispatchEvent::SetTitle. xterm caps title
            // length at ~1024 chars; we follow that convention.
            let title: String = if params.chars().count() > 1024 {
                params.chars().take(1024).collect()
            } else {
                params.to_string()
            };
            OscOutcome::Event(DispatchEvent::SetTitle(title))
        }
        "1" => OscOutcome::Drop, // icon name only — silently ignore
        "1338" => {
            let mut full = Vec::with_capacity(body.len() + 3);
            full.push(0x1B);
            full.push(b']');
            full.extend_from_slice(body);
            full.push(0x07);
            match vibeflow_protocol::parse(&full) {
                Ok(frame) => OscOutcome::Event(DispatchEvent::AiState(frame)),
                Err(_) => OscOutcome::Drop,
            }
        }
        "133" => match parse_133_body(params) {
            Some(marker) => OscOutcome::Event(DispatchEvent::Prompt(marker)),
            None => OscOutcome::Drop,
        },
        "52" => match parse_52_body(params) {
            Osc52ParseOutcome::Write { selection, text, truncated } => {
                if truncated {
                    tracing::warn!(
                        cap = MAX_OSC52_RAW_BYTES,
                        "OSC 52 write payload exceeded cap; truncated"
                    );
                }
                OscOutcome::Event(DispatchEvent::Osc52Write { selection, text })
            }
            Osc52ParseOutcome::ReadIgnored => {
                tracing::debug!("OSC 52 read request ignored (security)");
                OscOutcome::Drop
            }
            Osc52ParseOutcome::Malformed => {
                tracing::debug!("OSC 52 body malformed; dropping");
                OscOutcome::Drop
            }
        },
        _ => OscOutcome::Forward,
    }
}

impl Default for OscDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_133_prompt_start() {
        assert_eq!(parse_133_body("A"), Some(PromptMarker::PromptStart));
    }

    #[test]
    fn parse_133_prompt_end() {
        assert_eq!(parse_133_body("B"), Some(PromptMarker::PromptEnd));
    }

    #[test]
    fn parse_133_command_start() {
        assert_eq!(parse_133_body("C"), Some(PromptMarker::CommandStart));
    }

    #[test]
    fn parse_133_command_end_no_exit_code() {
        assert_eq!(
            parse_133_body("D"),
            Some(PromptMarker::CommandEnd { exit_code: None })
        );
    }

    #[test]
    fn parse_133_command_end_with_exit_code() {
        assert_eq!(
            parse_133_body("D;127"),
            Some(PromptMarker::CommandEnd {
                exit_code: Some(127)
            })
        );
    }

    #[test]
    fn parse_133_ignores_aid_on_prompt_start() {
        // iTerm's OSC 133;A;aid=<some-id> — we accept the subtype, ignore the aid
        assert_eq!(
            parse_133_body("A;aid=abc123"),
            Some(PromptMarker::PromptStart)
        );
    }

    #[test]
    fn parse_133_unknown_subtype_returns_none() {
        assert_eq!(parse_133_body("Z"), None);
        assert_eq!(parse_133_body(""), None);
    }

    #[test]
    fn parse_133_garbage_exit_code_falls_back_to_none() {
        // Non-numeric exit code: spec is silent, defensive behaviour is to
        // accept the CommandEnd marker but with no exit code.
        assert_eq!(
            parse_133_body("D;notanumber"),
            Some(PromptMarker::CommandEnd { exit_code: None })
        );
    }

    #[test]
    fn dispatcher_passes_plain_text_through_unchanged() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"hello, world");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(b"hello, world".to_vec())]
        );
    }

    #[test]
    fn dispatcher_passes_empty_input_through_with_no_events() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"");
        assert_eq!(events, vec![]);
    }

    #[test]
    fn dispatcher_passes_through_lone_esc_at_end_of_buffer() {
        // ESC at the end of a chunk is held internally, not emitted yet — but
        // at this stage of the plan we don't yet have the "emit ESC if next
        // byte isn't `]`" path. The simplest behaviour: an ESC that doesn't
        // form an OSC introducer is held in internal state until the next
        // feed call resolves it. Test the "no OSC came after" path: an ESC
        // followed by a non-`]` byte in a SINGLE feed is just passthrough.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"a\x1bb"); // ESC followed by 'b' (not ']') — passthrough as-is
        assert_eq!(events, vec![DispatchEvent::PassThrough(b"a\x1bb".to_vec())]);
    }

    use vibeflow_protocol::State;

    #[test]
    fn dispatcher_recognises_osc_1338_bel_terminated() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=working\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(Frame::new(State::Working))]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_1338_with_tool_and_project() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=waiting;tool=claude;project=vibeflow\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(
                Frame::new(State::Waiting)
                    .with_tool("claude")
                    .with_project("vibeflow")
            )]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_1338_st_terminated() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=active\x1b\\");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(Frame::new(State::Active))]
        );
    }

    #[test]
    fn dispatcher_emits_passthrough_around_osc_1338() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"hello\x1b]1338;state=working\x07world");
        assert_eq!(
            events,
            vec![
                DispatchEvent::PassThrough(b"hello".to_vec()),
                DispatchEvent::AiState(Frame::new(State::Working)),
                DispatchEvent::PassThrough(b"world".to_vec()),
            ]
        );
    }

    #[test]
    fn dispatcher_handles_double_esc_followed_by_osc() {
        // ESC ESC ] is "first ESC was a false start, second ESC is the real
        // introducer". The first ESC should land in passthrough; the OSC
        // should still be recognised.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b\x1b]1338;state=working\x07");
        assert_eq!(
            events,
            vec![
                DispatchEvent::PassThrough(b"\x1b".to_vec()),
                DispatchEvent::AiState(Frame::new(State::Working)),
            ]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_133_prompt_start() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;A\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::Prompt(PromptMarker::PromptStart)]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_133_command_start() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;C\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::Prompt(PromptMarker::CommandStart)]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_133_command_end_with_exit_code() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;D;127\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::Prompt(PromptMarker::CommandEnd {
                exit_code: Some(127)
            })]
        );
    }

    #[test]
    fn dispatcher_drops_osc_133_with_unknown_subtype() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;Z\x07");
        // No event; OSC 133 with unknown subtype is recognised-and-dropped.
        // Task 5 will distinguish this from completely unknown OSCs (which
        // become PassThrough).
        assert_eq!(events, vec![]);
    }

    #[test]
    fn dispatcher_passes_through_unknown_osc_intact() {
        // Use a genuinely unrecognised OSC ID. Stage 9 added OSC 0/2 as the
        // window-title sequence so they're no longer unknown — pick 999.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]999;garbage\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(b"\x1b]999;garbage\x07".to_vec())]
        );
    }

    #[test]
    fn dispatcher_passes_unknown_osc_with_st_terminator_intact() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]7;file://example\x1b\\");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(
                b"\x1b]7;file://example\x1b\\".to_vec()
            )]
        );
    }

    #[test]
    fn dispatcher_drops_oversize_osc() {
        let mut d = OscDispatcher::new();
        // Build a single OSC 1338 sequence whose body is well over MAX_OSC_LEN.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b]1338;state=waiting;tool=");
        input.extend(std::iter::repeat(b'x').take(5000));
        input.push(0x07);
        let events = d.feed(&input);
        // Oversize → silently dropped; no events.
        assert_eq!(events, vec![]);
    }

    #[test]
    fn dispatcher_handles_osc_split_across_two_feeds() {
        let mut d = OscDispatcher::new();
        let first = d.feed(b"hello\x1b]1338;state=");
        // The "hello" passthrough flushes at end of feed; the OSC body has
        // started and stays in internal state.
        assert_eq!(first, vec![DispatchEvent::PassThrough(b"hello".to_vec())]);
        let second = d.feed(b"working\x07world");
        assert_eq!(
            second,
            vec![
                DispatchEvent::AiState(Frame::new(State::Working)),
                DispatchEvent::PassThrough(b"world".to_vec()),
            ]
        );
    }

    #[test]
    fn dispatcher_recovers_from_malformed_osc() {
        // ESC `inside` an OSC body that doesn't form ST → drop the current
        // OSC and start a fresh OSC parse from the new ESC. The new OSC
        // (state=waiting) parses cleanly.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=garbage\x1b]1338;state=waiting\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(Frame::new(State::Waiting))]
        );
    }

    #[test]
    fn osc_0_emits_set_title() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]0;hello\x07");
        assert_eq!(events.len(), 1);
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "hello"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    #[test]
    fn osc_2_emits_set_title() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]2;world\x07");
        assert_eq!(events.len(), 1);
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "world"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    #[test]
    fn osc_0_with_embedded_semicolon_in_title() {
        let mut d = OscDispatcher::new();
        // OSC 0 has a single parameter — `;` chars after the first are part
        // of the title.
        let events = d.feed(b"\x1b]0;a;b;c\x07");
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "a;b;c"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    #[test]
    fn osc_1_is_silently_ignored() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1;icon\x07");
        for ev in &events {
            assert!(
                !matches!(ev, DispatchEvent::SetTitle(_)),
                "OSC 1 should not emit SetTitle, got {events:?}"
            );
        }
    }

    #[test]
    fn osc_0_with_st_terminator_works() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]0;st_form\x1b\\");
        match &events[0] {
            DispatchEvent::SetTitle(s) => assert_eq!(s, "st_form"),
            other => panic!("expected SetTitle, got {other:?}"),
        }
    }

    use proptest::prelude::*;

    #[test]
    fn parse_selection_c_only() {
        assert_eq!(parse_selection("c"), Osc52Selection::Clipboard);
    }

    #[test]
    fn parse_selection_p_only() {
        assert_eq!(parse_selection("p"), Osc52Selection::Primary);
    }

    #[test]
    fn parse_selection_s_means_both() {
        assert_eq!(parse_selection("s"), Osc52Selection::Both);
    }

    #[test]
    fn parse_selection_cp_means_both() {
        assert_eq!(parse_selection("cp"), Osc52Selection::Both);
    }

    #[test]
    fn parse_selection_pc_means_both() {
        assert_eq!(parse_selection("pc"), Osc52Selection::Both);
    }

    #[test]
    fn parse_selection_empty_defaults_to_clipboard() {
        assert_eq!(parse_selection(""), Osc52Selection::Clipboard);
    }

    #[test]
    fn parse_selection_unknown_letters_filtered() {
        assert_eq!(parse_selection("x"), Osc52Selection::Clipboard);
    }

    #[test]
    fn parse_selection_s_with_other_letters_short_circuits_to_both() {
        assert_eq!(parse_selection("cs"), Osc52Selection::Both);
    }

    proptest! {
        /// Feeding arbitrary bytes through the dispatcher in arbitrary chunk
        /// sizes must never panic and must never produce more bytes of
        /// PassThrough output than were fed in (the dispatcher has no source
        /// of expansion: every byte either feeds an OSC body, becomes part of
        /// a passthrough, or is dropped via overflow).
        #[test]
        fn dispatcher_never_panics_on_arbitrary_input(
            chunks in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..200),
                0..10,
            ),
        ) {
            let mut d = OscDispatcher::new();
            let mut total_input: usize = 0;
            let mut total_passthrough: usize = 0;
            for chunk in &chunks {
                total_input += chunk.len();
                for ev in d.feed(chunk) {
                    if let DispatchEvent::PassThrough(bytes) = ev {
                        total_passthrough += bytes.len();
                    }
                }
            }
            // PassThrough output cannot exceed total bytes fed in. (Equality
            // when no OSC was recognised; less when OSC bodies were consumed.)
            prop_assert!(total_passthrough <= total_input);
        }
    }

    #[test]
    fn parse_52_body_write_clipboard_base64() {
        let outcome = parse_52_body("c;SGVsbG8=");
        match outcome {
            Osc52ParseOutcome::Write { selection, text, truncated } => {
                assert_eq!(selection, Osc52Selection::Clipboard);
                assert_eq!(text, "Hello");
                assert!(!truncated);
            }
            _ => panic!("expected Write, got {:?}", outcome),
        }
    }

    #[test]
    fn parse_52_body_write_primary() {
        let outcome = parse_52_body("p;SGk=");
        match outcome {
            Osc52ParseOutcome::Write { selection, text, .. } => {
                assert_eq!(selection, Osc52Selection::Primary);
                assert_eq!(text, "Hi");
            }
            _ => panic!("expected Write, got {:?}", outcome),
        }
    }

    #[test]
    fn parse_52_body_write_both_via_s() {
        let outcome = parse_52_body("s;eHg=");
        match outcome {
            Osc52ParseOutcome::Write { selection, .. } => {
                assert_eq!(selection, Osc52Selection::Both);
            }
            _ => panic!("expected Write, got {:?}", outcome),
        }
    }

    #[test]
    fn parse_52_body_read_returns_ignored() {
        let outcome = parse_52_body("c;?");
        assert!(matches!(outcome, Osc52ParseOutcome::ReadIgnored));
    }

    #[test]
    fn parse_52_body_malformed_base64() {
        let outcome = parse_52_body("c;not_base64!");
        assert!(matches!(outcome, Osc52ParseOutcome::Malformed));
    }

    #[test]
    fn parse_52_body_no_separator() {
        let outcome = parse_52_body("c");
        assert!(matches!(outcome, Osc52ParseOutcome::Malformed));
    }

    #[test]
    fn parse_52_body_oversize_truncated() {
        use base64::Engine;
        let raw = "A".repeat(200 * 1024);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let body = format!("c;{}", encoded);
        let outcome = parse_52_body(&body);
        match outcome {
            Osc52ParseOutcome::Write { text, truncated, .. } => {
                assert_eq!(text.len(), 100 * 1024);
                assert!(truncated);
            }
            _ => panic!("expected truncated Write, got {:?}", outcome),
        }
    }

    #[test]
    fn dispatcher_emits_osc52write_for_full_sequence() {
        // Full byte sequence: ESC ] 52 ; c ; SGVsbG8= BEL
        let bytes = b"\x1b]52;c;SGVsbG8=\x07";
        let mut dispatcher = OscDispatcher::new();
        let events = dispatcher.feed(bytes);
        assert_eq!(events.len(), 1, "events: {:?}", events);
        match &events[0] {
            DispatchEvent::Osc52Write { selection, text } => {
                assert_eq!(*selection, Osc52Selection::Clipboard);
                assert_eq!(text, "Hello");
            }
            other => panic!("expected Osc52Write, got {:?}", other),
        }
    }

    #[test]
    fn dispatcher_drops_osc52_read_silently() {
        let bytes = b"\x1b]52;c;?\x07";
        let mut dispatcher = OscDispatcher::new();
        let events = dispatcher.feed(bytes);
        for ev in &events {
            assert!(
                !matches!(ev, DispatchEvent::Osc52Write { .. }),
                "no Osc52Write for read request, got {:?}", ev
            );
            assert!(
                !matches!(ev, DispatchEvent::PassThrough(_)),
                "no PassThrough for ignored read request, got {:?}", ev
            );
        }
    }

    #[test]
    fn dispatcher_emits_osc52write_under_max_osc_len() {
        use base64::Engine;
        let raw = "A".repeat(90 * 1024);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let mut bytes: Vec<u8> = Vec::with_capacity(128 * 1024);
        bytes.extend_from_slice(b"\x1b]52;c;");
        bytes.extend_from_slice(encoded.as_bytes());
        bytes.push(0x07);
        let mut dispatcher = OscDispatcher::new();
        let events = dispatcher.feed(&bytes);
        assert_eq!(events.len(), 1, "events count");
        match &events[0] {
            DispatchEvent::Osc52Write { text, .. } => {
                assert_eq!(text.len(), 90 * 1024, "text length unchanged when under cap");
            }
            other => panic!("expected Osc52Write, got {:?}", other),
        }
    }
}
