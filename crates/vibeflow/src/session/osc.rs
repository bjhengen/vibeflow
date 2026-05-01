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
#[allow(dead_code)] // first caller arrives in Task 4 (OscDispatcher OSC 133 detection)
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
const MAX_OSC_LEN: usize = 4096;

/// One event emitted by [`OscDispatcher::feed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchEvent {
    /// A complete OSC 1338 frame was parsed.
    AiState(Frame),
    /// An OSC 133 prompt marker was identified.
    Prompt(PromptMarker),
    /// Bytes that should be forwarded to the terminal grid (alacritty_terminal in
    /// future stages). Includes any unknown OSC sequences (their original bytes,
    /// terminator and all) plus all non-OSC bytes.
    PassThrough(Vec<u8>),
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
        self.state = ParseState::Plain;

        if overflowed {
            // Spec: "over-long sequences are dropped on the floor". No event.
            return;
        }

        if let Some(event) = handle_osc(&body) {
            events.push(event);
        }
        // If `handle_osc` returned None, that's a malformed-or-unknown OSC.
        // For now (Task 3) we drop. Task 5 reintroduces unknown-OSC
        // pass-through.
    }
}

/// Route a complete OSC body (the bytes between `ESC ]` and the terminator) to
/// the appropriate handler. Returns `None` for unknown OSCs and for OSC 1338
/// sequences that fail to parse.
fn handle_osc(body: &[u8]) -> Option<DispatchEvent> {
    let body_str = std::str::from_utf8(body).ok()?;
    let (id, _params) = body_str.split_once(';').unwrap_or((body_str, ""));
    match id {
        "1338" => {
            // Reconstruct the full sequence and hand it to the protocol crate.
            let mut full = Vec::with_capacity(body.len() + 3);
            full.push(0x1B);
            full.push(b']');
            full.extend_from_slice(body);
            full.push(0x07);
            vibeflow_protocol::parse(&full)
                .ok()
                .map(DispatchEvent::AiState)
        }
        _ => None,
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
}
