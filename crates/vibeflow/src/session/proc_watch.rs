//! Linux foreground-process detection for Tier 3 heuristic AI-tool awareness.
//!
//! Reads `/proc/<child_pid>/stat` to find the foreground process group of the
//! controlling terminal (field 8, `tpgid`), then reads `/proc/<tpgid>/comm`
//! to get the process name. Linux-only; non-Linux targets get a stub that
//! always returns `None`.
//!
//! Pure logic where possible: `parse_tpgid` is exposed for testing without I/O.

/// Generic language runtimes whose own `comm` does NOT name the tool they run
/// — the tool is a script/argument (e.g. `node /usr/bin/codex`, whose
/// foreground `comm` is "node"). For these we additionally consider the
/// launcher's argument basenames as candidate tool names (see
/// [`candidate_names`]), so wrapper-installed CLIs resolve to their real name.
const INTERPRETER_COMMS: &[&str] = &[
    "node", "deno", "bun", "python", "python2", "python3", "ruby", "perl", "php",
];

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

/// Tier-3 candidate tool names for the foreground process: always the
/// process-group leader's `comm`, plus — when that leader is a generic
/// interpreter (node/python/…) — the interpreter-resolved launcher names from
/// its command line. This lets a wrapper-installed CLI such as `codex` (run as
/// `node /usr/bin/codex`, leader `comm` = "node") be detected by its real name
/// without ever matching a bare `node` process: the caller still only matches
/// these candidates against the user's configured `[ai] tools` list.
///
/// Returns an empty `Vec` on non-Linux or when there's no foreground group.
pub fn foreground_command_candidates(child_pid: i32) -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let Some(tpgid) = foreground_pgid(child_pid).filter(|t| *t > 0) else {
            return Vec::new();
        };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{tpgid}/comm")) else {
            return Vec::new();
        };
        let comm = comm.trim();
        // The cmdline read is only needed to resolve an interpreter's script
        // name, so skip it (one fewer /proc read per tick) for the common case
        // of a native foreground binary.
        if INTERPRETER_COMMS.contains(&comm) {
            let cmdline =
                std::fs::read_to_string(format!("/proc/{tpgid}/cmdline")).unwrap_or_default();
            candidate_names(comm, &cmdline)
        } else {
            vec![comm.to_owned()]
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = child_pid;
        Vec::new()
    }
}

/// Pure core of [`foreground_command_candidates`]: given a foreground process's
/// `comm` and its raw NUL-separated `/proc/<pid>/cmdline`, return the candidate
/// tool names. Always yields `comm` first. When `comm` is a known interpreter,
/// also yields the extension-stripped basenames of its non-flag arguments
/// (arg0 — the interpreter itself — is skipped). A plain `node server.js`
/// therefore yields `["node", "server"]`, which matches the AI-tools list only
/// if the user explicitly listed `node` or `server`.
fn candidate_names(comm: &str, cmdline_nul: &str) -> Vec<String> {
    let mut out = vec![comm.to_owned()];
    if INTERPRETER_COMMS.contains(&comm) {
        for arg in cmdline_nul.split('\0').skip(1) {
            if arg.is_empty() || arg.starts_with('-') {
                continue; // trailing empty token, or a flag like `--inspect`
            }
            if let Some(stem) = std::path::Path::new(arg)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                if !stem.is_empty() && !out.iter().any(|c| c == stem) {
                    out.push(stem.to_owned());
                }
            }
        }
    }
    out
}

/// Read `/proc/<child_pid>/stat` and return field 8 (`tpgid`: the foreground
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

/// Parse `tpgid` (field 8 in canonical 1-based proc(5) numbering: pid, comm,
/// state, ppid, pgrp, session, tty_nr, tpgid) from a
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
    fn candidate_names_native_binary_is_just_comm() {
        // A native foreground binary: comm names the tool, cmdline ignored.
        assert_eq!(candidate_names("codex", ""), vec!["codex"]);
        assert_eq!(candidate_names("opencode", "opencode\0"), vec!["opencode"]);
        assert_eq!(candidate_names("grok", "grok\0"), vec!["grok"]);
    }

    #[test]
    fn candidate_names_non_interpreter_ignores_cmdline() {
        // bash is not an interpreter in our set; its args must NOT be mined.
        assert_eq!(
            candidate_names("bash", "bash\0/usr/bin/codex\0"),
            vec!["bash"]
        );
    }

    #[test]
    fn candidate_names_node_wrapper_resolves_script() {
        // The real codex case: `node /usr/bin/codex`, comm "node".
        assert_eq!(
            candidate_names("node", "node\0/usr/bin/codex\0"),
            vec!["node".to_string(), "codex".to_string()]
        );
    }

    #[test]
    fn candidate_names_python_wrapper_resolves_script() {
        assert_eq!(
            candidate_names("python3", "python3\0/home/u/.local/bin/aider\0"),
            vec!["python3".to_string(), "aider".to_string()]
        );
    }

    #[test]
    fn candidate_names_skips_flags_and_strips_extension() {
        // `node --inspect server.js` → flag skipped, `.js` stripped.
        assert_eq!(
            candidate_names("node", "node\0--inspect\0server.js\0"),
            vec!["node".to_string(), "server".to_string()]
        );
    }

    #[test]
    fn candidate_names_interpreter_with_no_script_is_just_comm() {
        assert_eq!(candidate_names("node", "node\0"), vec!["node"]);
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
