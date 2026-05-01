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
}
