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
#[allow(dead_code)] // used in Task 3 (OSC body overflow detection)
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
#[allow(dead_code)] // InOsc and InOscEsc used in Task 3 (OSC sequence parsing)
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
    #[allow(dead_code)] // used in Task 3 (OSC sequence parsing)
    osc_body: Vec<u8>,
    /// Pending pass-through bytes accumulated since the last emitted event.
    /// Flushed at the end of each `feed` call (or when an OSC starts).
    pass_buf: Vec<u8>,
    /// True once the current OSC body has overflowed `MAX_OSC_LEN`. We keep
    /// scanning for the terminator but discard the body and emit nothing.
    #[allow(dead_code)] // used in Task 3 (OSC body overflow detection)
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

    /// Process a single byte. State transitions only — no allocation in the
    /// hot path beyond the single `pass_buf` push per non-OSC byte.
    fn step(&mut self, b: u8, _events: &mut Vec<DispatchEvent>) {
        match self.state {
            ParseState::Plain => {
                if b == 0x1B {
                    self.state = ParseState::SeenEsc;
                } else {
                    self.pass_buf.push(b);
                }
            }
            ParseState::SeenEsc => {
                // We deferred the ESC byte. At Stage 2 of this plan, OSC entry
                // (next byte is `]`) lands in Task 3; for now, any byte after
                // ESC just resolves back to plain pass-through with the ESC
                // restored.
                self.pass_buf.push(0x1B);
                self.pass_buf.push(b);
                self.state = ParseState::Plain;
            }
            ParseState::InOsc | ParseState::InOscEsc => {
                // OSC parsing arrives in Task 3; for Stage 2, this branch is
                // unreachable because we never enter InOsc.
                unreachable!("OSC parsing not implemented until Task 3");
            }
        }
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
}
