//! Per-tab session machinery: OSC dispatching, AI-state tracking.
//!
//! Stage 2 ships [`osc::OscDispatcher`] and [`tracker::AiStateTracker`]. Stage 3
//! adds a `pty` submodule that drives a real PTY child process and feeds its
//! output bytes through `OscDispatcher::feed`.

pub mod osc;
pub mod tracker;
