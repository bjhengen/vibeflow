//! OSC 1338 protocol — vibeflow's open standard for AI-tool state signalling.
//!
//! See `docs/protocol.md` in the workspace root for the canonical wire-format spec.

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
        Self { state, tool: None, project: None }
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
}

/// Returns true for bytes that must be percent-encoded in an OSC 1338 value:
/// control bytes (0x00–0x1F, 0x7F), `;`, `=`, `%`, and any non-ASCII byte.
#[inline]
#[allow(dead_code)]
fn needs_encoding(b: u8) -> bool {
    b < 0x20 || b == 0x7f || b == b';' || b == b'=' || b == b'%' || b > 0x7f
}

#[must_use]
#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[allow(dead_code)]
fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => unreachable!("hex_nibble: caller masked to 4 bits"),
    }
}

#[inline]
#[allow(dead_code)]
fn hex_value(b: u8) -> Result<u8, ParseError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(ParseError::BadEncoding),
    }
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
}
