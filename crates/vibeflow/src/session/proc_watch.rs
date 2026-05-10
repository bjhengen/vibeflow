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
#[allow(dead_code)]
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

/// Parse `tpgid` (field 7 in canonical proc(5) numbering) from a
/// `/proc/<pid>/stat` line. The trick: `comm` (field 2) is paren-wrapped
/// and may itself contain `(`, `)`, or whitespace, so split-from-the-start
/// is wrong. Find the LAST `)` and operate on the suffix; tpgid is the 6th
/// whitespace-separated token in that suffix (state, ppid, pgrp, session,
/// tty_nr, tpgid).
#[allow(dead_code)]
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
        // bash, etc). We can't pin the exact name, but we can assert the
        // result is Some(non-empty).
        let pid = std::process::id() as i32;
        let name = foreground_command_name(pid);
        assert!(
            name.is_some(),
            "self should resolve to some foreground command"
        );
        let name = name.unwrap();
        assert!(!name.is_empty(), "comm should not be empty");
        // comm is kernel-truncated to 15 chars max.
        assert!(
            name.len() <= 15,
            "comm length {} exceeds kernel cap",
            name.len()
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn foreground_command_name_returns_none_on_non_linux() {
        // Stub returns None unconditionally on non-Linux targets.
        assert_eq!(foreground_command_name(std::process::id() as i32), None);
    }
}
