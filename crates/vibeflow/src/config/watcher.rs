//! Background file-watcher thread. Uses `notify` to detect changes to
//! `~/.config/vibeflow/config.toml`, debounces 250 ms, parses + validates,
//! and ships `AppUserEvent::ConfigReloaded` via `EventLoopProxy::send_event`
//! to the main thread.
