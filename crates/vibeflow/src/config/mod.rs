//! TOML configuration: schema types, parsing, hot-reload, and the
//! `AppUserEvent` enum delivered via `EventLoopProxy::send_event` from the
//! file-watcher thread to `WindowApp::user_event` on the main thread.

pub mod error_banner;
pub mod schema;
pub mod watcher;
