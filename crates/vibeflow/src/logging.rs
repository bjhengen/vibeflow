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

/// File log retention in days. Files matching `vibeflow.log.YYYY-MM-DD` with
/// mtime older than this are deleted at init time (best effort).
const LOG_RETENTION_DAYS: u64 = 7;

/// Best-effort cleanup: scan `state_dir` for `vibeflow.log.YYYY-MM-DD` files
/// older than `LOG_RETENTION_DAYS` and delete them. Errors are swallowed
/// silently — retention is never fatal.
fn cleanup_old_logs(state_dir: &std::path::Path) {
    let Some(cutoff) = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(LOG_RETENTION_DAYS * 86_400))
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return; // dir missing or unreadable; best-effort.
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        // Strict pattern: "vibeflow.log." + exactly 10 chars of YYYY-MM-DD.
        // Anything else (vibeflow.log.bak, README, etc.) is left alone.
        let Some(rest) = name_str.strip_prefix("vibeflow.log.") else {
            continue;
        };
        if rest.len() != 10
            || !rest.chars().enumerate().all(|(i, c)| match i {
                4 | 7 => c == '-',
                _ => c.is_ascii_digit(),
            })
        {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialise vibeflow's logging subsystem: a quiet stderr layer
/// (`vibeflow=warn` by default) + a rotated file log at
/// `$XDG_STATE_HOME/vibeflow/vibeflow.log.YYYY-MM-DD` with non-blocking writes
/// and 7-day retention.
///
/// `RUST_LOG` overrides both layers' default filters when set.
///
/// Returns the `WorkerGuard` for the non-blocking file writer. The caller MUST
/// keep the guard in scope for the lifetime of the program — dropping it
/// flushes pending writes and shuts down the worker thread.
///
/// # Errors
/// Returns Err only if the state directory cannot be located or created
/// (extremely rare). On Err, the caller is expected to fall back to a
/// stderr-only subscriber so vibeflow remains usable.
pub fn init() -> Result<WorkerGuard> {
    let dir = state_dir().context("locate XDG state dir for vibeflow logs")?;
    ensure_state_dir(&dir)?;
    cleanup_old_logs(&dir);

    let file_appender = tracing_appender::rolling::daily(&dir, "vibeflow.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let stderr_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("vibeflow=warn"));
    let file_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("vibeflow=info,warn"));

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr)
                .with_filter(stderr_filter),
        )
        .with(
            fmt::layer()
                .compact()
                .with_ansi(false)
                .with_writer(file_writer)
                .with_filter(file_filter),
        )
        .init();

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::time::{Duration, SystemTime};

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

    fn write_file_with_mtime(dir: &std::path::Path, name: &str, age: Duration) {
        let path = dir.join(name);
        let mut f = File::create(&path).expect("create");
        writeln!(f, "log content").unwrap();
        let mtime = SystemTime::now()
            .checked_sub(age)
            .expect("SystemTime::sub doesn't underflow for test ages");
        f.set_modified(mtime).expect("set_modified (Rust 1.75+)");
    }

    #[test]
    fn cleanup_deletes_logs_older_than_retention() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path();
        write_file_with_mtime(
            dir,
            "vibeflow.log.2024-01-01",
            Duration::from_secs(30 * 86_400),
        ); // 30 days old
        write_file_with_mtime(dir, "vibeflow.log.2026-05-23", Duration::from_secs(60)); // 1 min old
        cleanup_old_logs(dir);
        assert!(
            !dir.join("vibeflow.log.2024-01-01").exists(),
            "old file should be deleted"
        );
        assert!(
            dir.join("vibeflow.log.2026-05-23").exists(),
            "recent file should remain"
        );
    }

    #[test]
    fn cleanup_ignores_files_not_matching_prefix() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path();
        write_file_with_mtime(dir, "README", Duration::from_secs(30 * 86_400));
        write_file_with_mtime(dir, "vibeflow.log.bak", Duration::from_secs(30 * 86_400));
        write_file_with_mtime(dir, "vibeflow.log.2026-05-23", Duration::from_secs(60));
        cleanup_old_logs(dir);
        assert!(dir.join("README").exists(), "README untouched");
        assert!(
            dir.join("vibeflow.log.bak").exists(),
            "vibeflow.log.bak does not match strict pattern"
        );
        assert!(
            dir.join("vibeflow.log.2026-05-23").exists(),
            "recent log file untouched"
        );
    }

    #[test]
    fn cleanup_handles_empty_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Function returns unit; the test passes if it doesn't panic.
        cleanup_old_logs(temp.path());
    }

    #[test]
    fn cleanup_handles_nonexistent_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("does-not-exist");
        // Best-effort: should NOT panic on a missing dir.
        cleanup_old_logs(&missing);
    }
}
