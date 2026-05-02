//! Per-tab session machinery: PTY plumbing, OSC dispatching, AI-state tracking.

pub mod osc;
pub mod pty;
#[allow(clippy::module_inception)]
pub mod session;
pub mod tracker;

pub use session::{PtySession, SessionEvent};
