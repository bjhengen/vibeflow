//! Vibeflow's logging facility: a quiet stderr layer + a rotated file log.
//!
//! Stage 9 deferred this; v0.1.2 (Spec A') implements it.
//!
//! See `docs/superpowers/specs/2026-05-23-vibeflow-v0.1.2-logging-design.md`.

use std::path::PathBuf;

/// Returns `$XDG_STATE_HOME/vibeflow/` (default: `~/.local/state/vibeflow/`).
/// Returns `None` only if neither `$XDG_STATE_HOME` nor `$HOME` is set —
/// extremely rare on Linux; non-fatal (caller falls back to stderr-only).
fn state_dir() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("vibeflow"))
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
}
