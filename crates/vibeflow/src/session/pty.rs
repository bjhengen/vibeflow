//! Thin wrapper around `portable-pty`. Exposes [`spawn_pty`] which returns a
//! [`PtyHandles`] containing the bits the caller needs to drive a child process
//! on a pseudoterminal: the master reader, the master writer, and the child
//! process handle for liveness checks and explicit kill.
