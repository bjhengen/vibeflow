//! Background file-watcher thread. Uses `notify` to detect changes to
//! `~/.config/vibeflow/config.toml`, debounces 250 ms, parses + validates,
//! and ships `AppUserEvent::ConfigReloaded` via `EventLoopProxy::send_event`
//! to the main thread.

use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use winit::event_loop::EventLoopProxy;

use crate::config::{AppUserEvent, Config, ConfigError};

const DEBOUNCE: Duration = Duration::from_millis(250);

/// Spawn the watcher thread. Returns its `JoinHandle` for shutdown sequencing
/// (or just drop it; the thread exits naturally when the proxy fails to send,
/// which happens once the main event loop has exited).
///
/// The thread watches the `path`'s parent directory so deletes + recreates
/// of the file are seen.
///
/// # Errors
/// Returns `notify::Error` if the watcher fails to bind to the parent dir.
pub fn spawn(path: PathBuf, proxy: EventLoopProxy<AppUserEvent>) -> notify::Result<JoinHandle<()>> {
    let (tx, rx) = channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    let watch_dir = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    // The dir must exist; otherwise we silently no-op-spawn the thread (it
    // will idle waiting for events that never arrive).
    if watch_dir.exists() {
        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;
    }

    let handle = thread::Builder::new()
        .name("vibeflow-config-watcher".to_string())
        .spawn(move || {
            // Hold the watcher in scope so it isn't dropped while the thread runs.
            let _watcher = watcher;
            let mut deadline: Option<Instant> = None;
            loop {
                let timeout = deadline
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::from_secs(60));
                match rx.recv_timeout(timeout) {
                    Ok(Ok(event)) => {
                        if event_concerns(&event, &path) {
                            // Pure decision (see `decide`): Remove cancels any
                            // pending debounced reload (which would otherwise
                            // read defaults + empty errors and clear the banner
                            // we're about to raise); Create/Modify (re)arms the
                            // debounce and lets the timeout branch fire it.
                            let (action, new_deadline) = decide(&event.kind, Instant::now());
                            deadline = new_deadline;
                            if action == WatchAction::RaiseRemovedError {
                                let err = ConfigError::IoError(format!(
                                    "{} removed at runtime",
                                    path.display()
                                ));
                                if proxy.send_event(AppUserEvent::ConfigError(err)).is_err() {
                                    return; // event loop dropped → exit thread
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "watcher error");
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // Either debounce expired or idle 60s wakeup.
                        if let Some(d) = deadline {
                            if Instant::now() >= d {
                                deadline = None;
                                let (cfg, errors) = Config::load(&path);
                                if proxy
                                    .send_event(AppUserEvent::ConfigReloaded {
                                        config: Box::new(cfg),
                                        errors,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        })?;
    Ok(handle)
}

/// What the watcher loop should do for a concerning fs event.
#[derive(Debug, PartialEq, Eq)]
enum WatchAction {
    /// File removed: raise the error banner now (and cancel any pending reload).
    RaiseRemovedError,
    /// Create/Modify: (re)arm the debounce; no reload fires yet.
    ArmDebounce,
}

/// Decide what a *concerning* fs event means for the debounce state machine,
/// and what the new debounce deadline should be. Pure and unit-testable — the
/// I/O (`send_event`, `Config::load`) stays in `spawn`. Crucially, a `Remove`
/// returns `None` for the deadline, cancelling any pending debounced reload so
/// it can't clear a just-raised "file removed" banner (the documented bug this
/// state machine exists to avoid).
fn decide(kind: &EventKind, now: Instant) -> (WatchAction, Option<Instant>) {
    if matches!(kind, EventKind::Remove(_)) {
        (WatchAction::RaiseRemovedError, None)
    } else {
        (WatchAction::ArmDebounce, Some(now + DEBOUNCE))
    }
}

/// Does this notify event concern our config file?
///
/// notify reports events at the parent-dir level on most platforms; the
/// `paths` field tells us which exact file was touched.
fn event_concerns(event: &Event, target: &std::path::Path) -> bool {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
            event.paths.iter().any(|p| p == target)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The watcher thread is timing-sensitive; we can only assert the spawn
    // call succeeds and that event_concerns filters correctly. End-to-end
    // file-modify-roundtrip is covered by an `#[ignore]` integration test.

    #[test]
    fn event_concerns_matches_target_path() {
        let target = PathBuf::from("/tmp/vibeflow_test/config.toml");
        let ev = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![target.clone()],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(event_concerns(&ev, &target));
    }

    #[test]
    fn event_concerns_rejects_other_paths() {
        let target = PathBuf::from("/tmp/vibeflow_test/config.toml");
        let other = PathBuf::from("/tmp/vibeflow_test/other.toml");
        let ev = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![other],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(!event_concerns(&ev, &target));
    }

    #[test]
    fn event_concerns_rejects_access_events() {
        let target = PathBuf::from("/tmp/vibeflow_test/config.toml");
        let ev = Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![target.clone()],
            attrs: notify::event::EventAttributes::new(),
        };
        assert!(!event_concerns(&ev, &target));
    }

    // The debounce / Remove-cancel decision is the one watcher path with a
    // documented past bug (a Remove must cancel a pending reload, or the
    // post-debounce reload reads defaults + empty errors and clears the
    // just-raised banner). It's extracted into the pure `decide` fn so the
    // transitions can be tested without a real EventLoop or inotify timing.

    #[test]
    fn decide_remove_cancels_pending_reload() {
        let now = Instant::now();
        let (action, deadline) = decide(&EventKind::Remove(notify::event::RemoveKind::Any), now);
        assert_eq!(action, WatchAction::RaiseRemovedError);
        assert_eq!(deadline, None, "Remove must clear the debounce deadline");
    }

    #[test]
    fn decide_modify_arms_debounce() {
        let now = Instant::now();
        let (action, deadline) = decide(&EventKind::Modify(notify::event::ModifyKind::Any), now);
        assert_eq!(action, WatchAction::ArmDebounce);
        assert_eq!(deadline, Some(now + DEBOUNCE));
    }

    #[test]
    fn decide_create_arms_debounce() {
        let now = Instant::now();
        let (action, deadline) = decide(&EventKind::Create(notify::event::CreateKind::Any), now);
        assert_eq!(action, WatchAction::ArmDebounce);
        assert_eq!(deadline, Some(now + DEBOUNCE));
    }
}
