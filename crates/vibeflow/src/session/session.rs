//! `PtySession` — one tab's PTY child, reader thread, OSC dispatcher, and
//! AI-state tracker, all driven from the main thread via a single-producer
//! single-consumer channel.
