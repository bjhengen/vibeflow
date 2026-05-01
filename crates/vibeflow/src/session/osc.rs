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
}
