//! Vibeflow's logging facility: a quiet stderr layer + a rotated file log.
//!
//! Stage 9 deferred this; v0.1.2 (Spec A') implements it.
//!
//! See `docs/superpowers/specs/2026-05-23-vibeflow-v0.1.2-logging-design.md`.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Returns `$XDG_STATE_HOME/vibeflow/` (default: `~/.local/state/vibeflow/`).
/// Returns `None` only if neither `$XDG_STATE_HOME` nor `$HOME` is set —
/// extremely rare on Linux; non-fatal (caller falls back to stderr-only).
fn state_dir() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("vibeflow"))
}

/// Ensure the state directory exists with 0o700 perms (owner-only access).
/// No-op if already present. Returns Err if creation fails (e.g. parent dir
/// permission denied); caller falls back to stderr-only logging.
fn ensure_state_dir(path: &std::path::Path) -> Result<()> {
    if path.exists() && path.is_dir() {
        // Already a directory; no-op.
        return Ok(());
    }
    if path.exists() {
        // Exists but is not a directory (e.g., a file).
        anyhow::bail!(
            "create vibeflow state dir {}: path exists but is not a directory",
            path.display()
        );
    }
    // Path does not exist; create it with perms.
    std::fs::create_dir_all(path)
        .with_context(|| format!("create vibeflow state dir {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("set perms on {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_dir_ends_with_vibeflow_subdir() {
        // `dirs::state_dir()` returns Some on Linux when $HOME (or
        // $XDG_STATE_HOME) is set, which is the case in `cargo test` env.
        let path = state_dir().expect("state_dir should resolve on Linux test runner");
        assert!(
            path.ends_with("vibeflow"),
            "state_dir should append 'vibeflow' subdir, got {}",
            path.display()
        );
    }

    #[test]
    fn ensure_state_dir_creates_with_700_perms() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("vibeflow");
        ensure_state_dir(&dir).expect("ensure_state_dir succeeds");
        assert!(dir.exists(), "dir created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "perms should be 0o700, got {:o}", mode);
        }
    }

    #[test]
    fn ensure_state_dir_no_op_when_dir_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("vibeflow");
        std::fs::create_dir_all(&dir).expect("pre-create");
        // Second call is a no-op (no error, no perm change attempt).
        ensure_state_dir(&dir).expect("ensure_state_dir no-op succeeds");
        assert!(dir.exists());
    }

    #[test]
    fn ensure_state_dir_returns_err_when_path_is_a_file() {
        // Covers the spec's "fallback_init_on_state_dir_failure" contract at the
        // helper level (init() itself is process-global and not unit-testable).
        // Pre-create a regular file where the dir would go; create_dir_all
        // must then fail.
        let temp = tempfile::tempdir().expect("tempdir");
        let blocked = temp.path().join("vibeflow");
        std::fs::write(&blocked, b"i am a file").expect("write blocker");
        let err = ensure_state_dir(&blocked).expect_err("must fail when path is a file");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("create vibeflow state dir"),
            "error should carry the anyhow context, got: {msg}"
        );
    }
}
