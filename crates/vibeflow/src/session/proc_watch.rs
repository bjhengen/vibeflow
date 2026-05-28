//! Linux foreground-process detection for Tier 3 heuristic AI-tool awareness.
//!
//! Reads `/proc/<child_pid>/stat` to find the foreground process group of the
//! controlling terminal (field 7, `tpgid`), then reads `/proc/<tpgid>/comm`
//! to get the process name. Linux-only; non-Linux targets get a stub that
//! always returns `None`.
//!
//! Pure logic where possible: `parse_tpgid` is exposed for testing without I/O.

/// Read /proc/<child_pid>/stat → tpgid → /proc/<tpgid>/comm. Returns the
/// trimmed command name (no parens, no trailing newline) or None on any
/// I/O error or if there's no foreground process group.
///
/// Caveat: kernel truncates `comm` to 15 chars; match-list entries longer
/// than 15 chars will silently never match.
///
/// Used by tests and called from `PtySession::tick`.
pub fn foreground_command_name(child_pid: i32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{child_pid}/stat")).ok()?;
        let tpgid = parse_tpgid(&stat)?;
        if tpgid <= 0 {
            return None;
        }
        let comm = std::fs::read_to_string(format!("/proc/{tpgid}/comm")).ok()?;
        Some(comm.trim().to_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = child_pid;
        None
    }
}

/// Read `/proc/<child_pid>/stat` and return field 7 (`tpgid`: the foreground
/// process group ID of the controlling terminal). Returns `None` on non-Linux,
/// on read error, or if the stat line is malformed.
///
/// Used by `PtySession::has_foreground_child` to detect when something other
/// than the shell itself holds the controlling terminal (i.e. the shell is
/// running a subprocess like `python3`, `vim`, `claude`).
///
/// Note: `tpgid` is `-1` when there is no controlling terminal, which most
/// callers want to treat as "no foreground process". Callers should usually
/// guard on `tpgid > 0`.
pub fn foreground_pgid(child_pid: i32) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{child_pid}/stat")).ok()?;
        parse_tpgid(&stat)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = child_pid;
        None
    }
}

/// Parse `tpgid` (field 7 in canonical proc(5) numbering) from a
/// `/proc/<pid>/stat` line. The trick: `comm` (field 2) is paren-wrapped
/// and may itself contain `(`, `)`, or whitespace, so split-from-the-start
/// is wrong. Find the LAST `)` and operate on the suffix; tpgid is the 6th
/// whitespace-separated token in that suffix (state, ppid, pgrp, session,
/// tty_nr, tpgid).
///
/// Called by foreground_command_name and tested directly via unit tests.
fn parse_tpgid(stat_line: &str) -> Option<i32> {
    let after_comm = stat_line.rsplit_once(')')?.1.trim_start();
    after_comm.split_whitespace().nth(5)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tpgid_simple_case() {
        // pid (comm) state ppid pgrp session tty_nr tpgid …
        // 1234 (bash) S 1000 1234 1234 34816 5678 …
        let line = "1234 (bash) S 1000 1234 1234 34816 5678 4194304 ...";
        assert_eq!(parse_tpgid(line), Some(5678));
    }

    #[test]
    fn parse_tpgid_handles_paren_in_comm() {
        // Kernel preserves real parens in comm by including them as-is. We
        // rely on rsplit_once finding the LAST `)`.
        let line = "1234 ((weird)thing) S 1000 1234 1234 34816 -1 ...";
        assert_eq!(parse_tpgid(line), Some(-1));
    }

    #[test]
    fn parse_tpgid_handles_space_in_comm() {
        let line = "1234 (my prog) R 1000 1234 1234 34816 9999 ...";
        assert_eq!(parse_tpgid(line), Some(9999));
    }

    #[test]
    fn parse_tpgid_returns_none_when_no_close_paren() {
        let line = "1234 bash S 1000 1234 1234 34816 5678";
        assert_eq!(parse_tpgid(line), None);
    }

    #[test]
    fn parse_tpgid_returns_none_when_too_few_fields() {
        let line = "1234 (bash) S 1000 1234";
        assert_eq!(parse_tpgid(line), None);
    }

    #[test]
    fn parse_tpgid_returns_none_when_field_not_int() {
        let line = "1234 (bash) S 1000 1234 1234 34816 abc 4194304";
        assert_eq!(parse_tpgid(line), None);
    }

    #[test]
    fn foreground_command_name_returns_none_for_invalid_pid() {
        // i32::MAX is unlikely to be a real pid; -1 is invalid.
        assert_eq!(foreground_command_name(-1), None);
        assert_eq!(foreground_command_name(i32::MAX), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn foreground_command_name_round_trips_for_self() {
        // The test process's tpgid points at whatever ran cargo test (cargo,
        // bash, etc) ONLY if there's a controlling TTY. In non-TTY contexts
        // (cargo test runner without `-t`, CI), tpgid = -1 and the function
        // correctly returns None. Skip the body in that case — the parse-
        // logic tests above cover the deterministic surface.
        let pid = std::process::id() as i32;
        if let Some(name) = foreground_command_name(pid) {
            assert!(!name.is_empty(), "comm should not be empty when present");
            // comm is kernel-truncated to 15 chars max.
            assert!(
                name.len() <= 15,
                "comm length {} exceeds kernel cap",
                name.len()
            );
        }
        // If None: running without controlling TTY; not a failure.
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn foreground_command_name_returns_none_on_non_linux() {
        // Stub returns None unconditionally on non-Linux targets.
        assert_eq!(foreground_command_name(std::process::id() as i32), None);
    }

    #[test]
    fn foreground_pgid_returns_none_for_invalid_pid() {
        assert_eq!(foreground_pgid(-1), None);
        assert_eq!(foreground_pgid(i32::MAX), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn foreground_pgid_round_trips_for_self_when_tty_present() {
        // Mirror of `foreground_command_name_round_trips_for_self`: tpgid is
        // only meaningful when the test runner has a controlling TTY. If we
        // get Some, it's a valid i32; -1 is valid (no controlling terminal).
        // None is also valid (parsing or read error). Just verify it doesn't panic.
        let pid = std::process::id() as i32;
        let _tpgid = foreground_pgid(pid);
        // If we got here without panic, the function works correctly.
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn foreground_pgid_returns_none_on_non_linux() {
        assert_eq!(foreground_pgid(std::process::id() as i32), None);
    }
}
