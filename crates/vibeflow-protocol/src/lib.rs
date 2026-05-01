//! OSC 1338 protocol — vibeflow's open standard for AI-tool state signalling.
//!
//! See `docs/protocol.md` in the workspace root for the canonical wire-format spec.

use std::io::Write;

/// `ESC` byte (start of an OSC sequence).
pub const ESC: u8 = 0x1B;
/// `BEL` byte (one of two valid OSC terminators).
pub const BEL: u8 = 0x07;
/// String-terminator (the second valid terminator) is `ESC \` — two bytes.
pub const ST: [u8; 2] = [ESC, b'\\'];
/// OSC 1338 sequences over this length are dropped on the floor.
pub const MAX_FRAME_LEN: usize = 4096;
/// The OSC identifier we own.
pub const OSC_ID: &str = "1338";

/// Per-tab AI-tool state, as carried in the `state` parameter of OSC 1338.
///
/// Variants are listed in order of "loudness" of the visual indicator:
/// `Active` is the default with no special styling; `Waiting` is the headline
/// state that pulses amber on the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    /// The default — nothing special is happening on this tab.
    Active,
    /// A tool is running / generating; tab shows a steady blue stripe.
    Working,
    /// A tool is waiting for user input; tab pulses amber. The headline state.
    Waiting,
    /// A tool just finished a task; usually a transient state that flips back to `Active`.
    Done,
}

impl State {
    /// The wire string for this state, as it appears in OSC 1338.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Working => "working",
            State::Waiting => "waiting",
            State::Done => "done",
        }
    }
}

impl std::str::FromStr for State {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(State::Active),
            "working" => Ok(State::Working),
            "waiting" => Ok(State::Waiting),
            "done" => Ok(State::Done),
            _ => Err(ParseError::UnknownState(s.to_owned())),
        }
    }
}

/// Errors produced when parsing OSC 1338 frames or state strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Bytes are not an OSC 1338 sequence (wrong prefix or wrong identifier).
    NotOurOsc,
    /// The required `state` key was absent.
    MissingState,
    /// The `state` value was not one of the four known variants.
    UnknownState(String),
    /// The sequence was structurally malformed (e.g., no terminator).
    Malformed(&'static str),
    /// The sequence exceeded the 4 KiB cap.
    TooLong,
    /// A percent-encoded byte was malformed.
    BadEncoding,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NotOurOsc => f.write_str("not an OSC 1338 sequence"),
            ParseError::MissingState => f.write_str("missing required `state` key"),
            ParseError::UnknownState(s) => write!(f, "unknown state value: {s}"),
            ParseError::Malformed(why) => write!(f, "malformed sequence: {why}"),
            ParseError::TooLong => f.write_str("sequence exceeds 4 KiB"),
            ParseError::BadEncoding => f.write_str("invalid percent encoding"),
        }
    }
}

impl std::error::Error for ParseError {}

/// A single OSC 1338 frame's contents.
///
/// Construct with [`Frame::new`] and chain [`Frame::with_tool`] / [`Frame::with_project`]:
///
/// ```
/// use vibeflow_protocol::{Frame, State};
/// let f = Frame::new(State::Waiting).with_tool("claude").with_project("vibeflow");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub state: State,
    pub tool: Option<String>,
    pub project: Option<String>,
}

impl Frame {
    /// A new frame with only the required `state` field set.
    #[must_use]
    pub fn new(state: State) -> Self {
        Self {
            state,
            tool: None,
            project: None,
        }
    }

    /// Set the optional `tool` field. Returns `self` for chaining.
    #[must_use]
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    /// Set the optional `project` field. Returns `self` for chaining.
    #[must_use]
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Serialise this frame as the bytes of an OSC 1338 sequence terminated by `BEL`.
    ///
    /// (BEL terminator chosen over ST because it's simpler and is what xterm/iTerm/most
    /// terminals emit themselves. Either is acceptable per the spec.)
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::with_capacity(64);
        s.push(ESC as char);
        s.push(']');
        s.push_str(OSC_ID);
        s.push(';');
        s.push_str("state=");
        s.push_str(self.state.as_str());
        if let Some(tool) = &self.tool {
            s.push_str(";tool=");
            s.push_str(&percent_encode(tool));
        }
        if let Some(project) = &self.project {
            s.push_str(";project=");
            s.push_str(&percent_encode(project));
        }
        s.push(BEL as char);
        s.into_bytes()
    }
}

/// Returns true for bytes that must be percent-encoded in an OSC 1338 value:
/// control bytes (0x00–0x1F, 0x7F), `;`, `=`, `%`, and any non-ASCII byte.
#[inline]
fn needs_encoding(b: u8) -> bool {
    b < 0x20 || b == 0x7f || b == b';' || b == b'=' || b == b'%' || b > 0x7f
}

#[must_use]
pub(crate) fn percent_encode(s: &str) -> String {
    let bytes = s.as_bytes();
    // Fast path: nothing to encode.
    if !bytes.iter().copied().any(needs_encoding) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if needs_encoding(b) {
            // %XX, uppercase hex (RFC 3986 convention).
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        } else {
            out.push(b as char);
        }
    }
    out
}

pub(crate) fn percent_decode(s: &str) -> Result<String, ParseError> {
    let bytes = s.as_bytes();
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(ParseError::BadEncoding);
            }
            let hi = hex_value(bytes[i + 1])?;
            let lo = hex_value(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ParseError::BadEncoding)
}

#[inline]
fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => unreachable!("hex_nibble: caller masked to 4 bits"),
    }
}

#[inline]
fn hex_value(b: u8) -> Result<u8, ParseError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(ParseError::BadEncoding),
    }
}

/// Parse a complete OSC 1338 frame from the byte slice.
///
/// The caller is responsible for delivering exactly one framed sequence — the
/// streaming `OscDispatcher` in the vibeflow binary slices bytes between
/// `ESC ]` and the next `BEL` / `ST` terminator before calling this.
///
/// # Errors
/// See [`ParseError`].
pub fn parse(bytes: &[u8]) -> Result<Frame, ParseError> {
    if bytes.len() > MAX_FRAME_LEN {
        return Err(ParseError::TooLong);
    }

    // Strip the OSC introducer: `ESC ]`.
    let rest = bytes
        .strip_prefix(&[ESC, b']'])
        .ok_or(ParseError::NotOurOsc)?;

    // Find and strip the terminator (BEL or ST).
    let body = strip_terminator(rest)?;

    // The body is `1338;k1=v1;k2=v2…` — must be valid UTF-8 (per spec).
    let body = std::str::from_utf8(body).map_err(|_| ParseError::Malformed("non-UTF-8 body"))?;

    let mut parts = body.split(';');
    let id = parts.next().ok_or(ParseError::Malformed("empty body"))?;
    if id != OSC_ID {
        return Err(ParseError::NotOurOsc);
    }

    let mut state: Option<State> = None;
    let mut tool: Option<String> = None;
    let mut project: Option<String> = None;

    for part in parts {
        // Split on the *first* `=` only — values may contain `=` if percent-encoded
        // would have escaped it, but a literal `=` in a malformed frame should
        // still parse cleanly to "key" + "value-with-equals".
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key {
            "state" => {
                let decoded = percent_decode(value)?;
                state = Some(decoded.parse()?);
            }
            "tool" => tool = Some(percent_decode(value)?),
            "project" => project = Some(percent_decode(value)?),
            _ => { /* unknown key — ignore for forward compatibility */ }
        }
    }

    let state = state.ok_or(ParseError::MissingState)?;
    Ok(Frame {
        state,
        tool,
        project,
    })
}

/// Locate either `BEL` or `ESC \` and return the body slice (everything before it).
fn strip_terminator(rest: &[u8]) -> Result<&[u8], ParseError> {
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            BEL => return Ok(&rest[..i]),
            b if b == ESC && rest.get(i + 1).copied() == Some(b'\\') => {
                return Ok(&rest[..i]);
            }
            _ => i += 1,
        }
    }
    Err(ParseError::Malformed("no terminator"))
}

/// Write the OSC 1338 byte sequence for `frame` to `writer`. Use this when you
/// need to write somewhere other than stdout (tests, files, sockets).
///
/// # Errors
/// Propagates any [`std::io::Error`] from the underlying writer.
pub fn emit_to<W: std::io::Write>(writer: &mut W, frame: &Frame) -> std::io::Result<()> {
    writer.write_all(&frame.to_bytes())
}

/// Write `frame` to stdout and flush.
///
/// # Errors
/// Returns the underlying I/O error if stdout cannot be written.
pub fn emit(frame: &Frame) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    emit_to(&mut handle, frame)?;
    handle.flush()
}

/// Convenience for `emit(&Frame::new(state))`.
///
/// # Errors
/// Returns the underlying I/O error if stdout cannot be written.
pub fn emit_state(state: State) -> std::io::Result<()> {
    emit(&Frame::new(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_as_str_round_trips_via_from_str() {
        for s in ["active", "working", "waiting", "done"] {
            let parsed: State = s.parse().expect("known state must parse");
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn state_unknown_string_is_an_error() {
        let err = "frobnicating".parse::<State>().unwrap_err();
        assert!(matches!(err, ParseError::UnknownState(ref s) if s == "frobnicating"));
    }

    #[test]
    fn frame_new_has_state_only() {
        let f = Frame::new(State::Working);
        assert_eq!(f.state, State::Working);
        assert_eq!(f.tool, None);
        assert_eq!(f.project, None);
    }

    #[test]
    fn frame_with_tool_and_project_builds_correctly() {
        let f = Frame::new(State::Waiting)
            .with_tool("claude")
            .with_project("vibeflow");
        assert_eq!(f.state, State::Waiting);
        assert_eq!(f.tool.as_deref(), Some("claude"));
        assert_eq!(f.project.as_deref(), Some("vibeflow"));
    }

    #[test]
    fn percent_encode_passes_safe_ascii_through() {
        assert_eq!(percent_encode("hello-world_42"), "hello-world_42");
    }

    #[test]
    fn percent_encode_escapes_specials() {
        assert_eq!(percent_encode("a;b=c"), "a%3Bb%3Dc");
        assert_eq!(percent_encode("100%"), "100%25");
    }

    #[test]
    fn percent_encode_escapes_non_ascii_as_utf8_bytes() {
        // "café" → c, a, f, é (0xC3 0xA9 in UTF-8)
        assert_eq!(percent_encode("café"), "caf%C3%A9");
    }

    #[test]
    fn percent_decode_roundtrips_arbitrary_strings() {
        for s in ["", "plain", "a;b=c", "café", "100%", "tab\there"] {
            let encoded = percent_encode(s);
            let decoded = percent_decode(&encoded).expect("round-trip");
            assert_eq!(decoded, s);
        }
    }

    #[test]
    fn percent_decode_rejects_truncated_escape() {
        assert_eq!(percent_decode("foo%2"), Err(ParseError::BadEncoding));
        assert_eq!(percent_decode("foo%"), Err(ParseError::BadEncoding));
    }

    #[test]
    fn percent_decode_rejects_non_hex_digits() {
        assert_eq!(percent_decode("foo%ZZ"), Err(ParseError::BadEncoding));
    }

    #[test]
    fn to_bytes_minimal_frame_is_state_only() {
        let bytes = Frame::new(State::Waiting).to_bytes();
        assert_eq!(bytes, b"\x1b]1338;state=waiting\x07");
    }

    #[test]
    fn to_bytes_with_tool_and_project() {
        let bytes = Frame::new(State::Working)
            .with_tool("claude")
            .with_project("vibeflow")
            .to_bytes();
        assert_eq!(
            bytes,
            b"\x1b]1338;state=working;tool=claude;project=vibeflow\x07"
        );
    }

    #[test]
    fn to_bytes_percent_encodes_special_characters_in_values() {
        let bytes = Frame::new(State::Active).with_tool("a;b=c").to_bytes();
        assert_eq!(bytes, b"\x1b]1338;state=active;tool=a%3Bb%3Dc\x07");
    }

    #[test]
    fn parse_minimal_bel_terminated() {
        let f = parse(b"\x1b]1338;state=waiting\x07").unwrap();
        assert_eq!(f, Frame::new(State::Waiting));
    }

    #[test]
    fn parse_minimal_st_terminated() {
        let f = parse(b"\x1b]1338;state=active\x1b\\").unwrap();
        assert_eq!(f, Frame::new(State::Active));
    }

    #[test]
    fn parse_full_frame_with_all_keys() {
        let f = parse(b"\x1b]1338;state=working;tool=claude;project=vibeflow\x07").unwrap();
        assert_eq!(
            f,
            Frame::new(State::Working)
                .with_tool("claude")
                .with_project("vibeflow")
        );
    }

    #[test]
    fn parse_decodes_percent_escapes_in_values() {
        let f = parse(b"\x1b]1338;state=active;tool=a%3Bb%3Dc\x07").unwrap();
        assert_eq!(f, Frame::new(State::Active).with_tool("a;b=c"));
    }

    #[test]
    fn parse_ignores_unknown_keys_for_forward_compat() {
        let f = parse(b"\x1b]1338;state=waiting;newfield=hello;tool=claude\x07").unwrap();
        assert_eq!(f, Frame::new(State::Waiting).with_tool("claude"));
    }

    #[test]
    fn parse_rejects_wrong_prefix() {
        assert_eq!(parse(b"hello\x07"), Err(ParseError::NotOurOsc));
        assert_eq!(
            parse(b"\x1b]133;state=waiting\x07"),
            Err(ParseError::NotOurOsc)
        );
    }

    #[test]
    fn parse_requires_state_key() {
        assert_eq!(
            parse(b"\x1b]1338;tool=claude\x07"),
            Err(ParseError::MissingState)
        );
    }

    #[test]
    fn parse_rejects_unknown_state_value() {
        match parse(b"\x1b]1338;state=zonking\x07") {
            Err(ParseError::UnknownState(ref s)) if s == "zonking" => {}
            other => panic!("expected UnknownState, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_terminator() {
        assert!(matches!(
            parse(b"\x1b]1338;state=waiting"),
            Err(ParseError::Malformed(_))
        ));
    }

    #[test]
    fn parse_rejects_oversized_input() {
        let mut big = Vec::with_capacity(MAX_FRAME_LEN + 100);
        big.extend_from_slice(b"\x1b]1338;state=waiting;tool=");
        big.extend(std::iter::repeat(b'x').take(MAX_FRAME_LEN));
        big.push(BEL);
        assert_eq!(parse(&big), Err(ParseError::TooLong));
    }

    use proptest::prelude::*;

    fn arb_state() -> impl Strategy<Value = State> {
        prop_oneof![
            Just(State::Active),
            Just(State::Working),
            Just(State::Waiting),
            Just(State::Done),
        ]
    }

    // Any UTF-8 string up to 100 chars (any non-newline scalar — proptest's
    // regex `.` excludes \n by default, which is fine; we cover `\t` etc.
    // explicitly in the unit tests).
    //
    // Why `string_regex` not a bare `".{0,100}"`: a `&str` literal does NOT
    // implement `Strategy<Value=String>`. The proptest! macro accepts string
    // literals after `in` because the macro converts them to a regex strategy
    // — but in plain function-form code we have to call `string_regex`
    // explicitly. The `.unwrap()` is fine because the regex is a literal.
    //
    // Why 100: worst-case encoding is 4 UTF-8 bytes per char × 3 chars per
    // percent-encoded byte = 12 chars per char in the wire form, so two such
    // values plus the rest of the frame fit comfortably under MAX_FRAME_LEN.
    fn arb_value() -> impl Strategy<Value = String> {
        proptest::string::string_regex(".{0,100}").unwrap()
    }

    fn arb_frame() -> impl Strategy<Value = Frame> {
        (
            arb_state(),
            proptest::option::of(arb_value()),
            proptest::option::of(arb_value()),
        )
            .prop_map(|(state, tool, project)| Frame {
                state,
                tool,
                project,
            })
    }

    proptest! {
        #[test]
        fn frame_to_bytes_then_parse_roundtrips(frame in arb_frame()) {
            let bytes = frame.to_bytes();
            let parsed = parse(&bytes).expect("round-trip should always parse");
            prop_assert_eq!(parsed, frame);
        }
    }

    #[test]
    fn emit_writes_to_provided_writer() {
        // emit_to is the seam we test against; emit() and emit_state() wrap it.
        let mut buf = Vec::<u8>::new();
        let f = Frame::new(State::Working).with_tool("claude");
        emit_to(&mut buf, &f).expect("write should succeed");
        assert_eq!(buf, b"\x1b]1338;state=working;tool=claude\x07");
    }
}
