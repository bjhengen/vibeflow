//! Thin wrapper around `portable-pty`. Exposes [`spawn_pty`] which returns a
//! [`PtyHandles`] containing the bits the caller needs to drive a child process
//! on a pseudoterminal: the master reader, the master writer, and the child
//! process handle for liveness checks and explicit kill.

use std::io::{Read, Write};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};

/// Handles returned from [`spawn_pty`]. The fields are owned by separate
/// threads in `PtySession`: the reader is moved into the reader thread,
/// the writer stays on the main thread, the child is owned by `PtySession`
/// for liveness checks and explicit kill, and the master must be kept alive
/// alongside the reader (its drop closes the PTY).
pub struct PtyHandles {
    /// Read half of the PTY master. Move to a reader thread.
    pub reader: Box<dyn Read + Send>,
    /// Write half of the PTY master. Used by the main thread for keyboard input.
    pub writer: Box<dyn Write + Send>,
    /// The child process. Drop or kill to terminate.
    pub child: Box<dyn Child + Send + Sync>,
    /// The master PTY. Keep alive as long as `reader` is in use — once the
    /// box is dropped, the PTY closes and reads return EOF. Callers should
    /// move it into the same scope as the reader (typically the reader thread).
    pub master: Box<dyn MasterPty + Send>,
}

/// Spawn a child process on a pseudoterminal.
///
/// `argv` is the command + arguments — `argv[0]` is the program path. PTY size
/// defaults to 80x24; resizing is added in Stage 6 (window event handler).
///
/// # Errors
/// Returns an `io::Error` if the PTY cannot be opened or the child cannot be
/// spawned. Wraps `portable_pty`'s typed errors via `io::Error::other`.
pub fn spawn_pty(argv: &[&str]) -> std::io::Result<PtyHandles> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(std::io::Error::other)?;

    let mut cmd = CommandBuilder::new(argv[0]);
    for arg in &argv[1..] {
        cmd.arg(arg);
    }
    // Set TERM so children behave reasonably. `xterm-256color` is a safe
    // baseline; Stage 6 may switch to `vibeflow` once we register a terminfo.
    cmd.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(std::io::Error::other)?;
    // Drop the slave so the master is the only end of the PTY — reads will
    // see EOF only when the child exits.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(std::io::Error::other)?;
    let writer = pair.master.take_writer().map_err(std::io::Error::other)?;

    Ok(PtyHandles {
        reader,
        writer,
        child,
        master: pair.master,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spawn_sh_echo_reads_back_the_string() {
        // Spawn `sh -c "printf hello"`. The child writes "hello" to stdout
        // (which is the PTY slave), then exits. We read from the master.
        let handles = spawn_pty(&["/bin/sh", "-c", "printf hello"]).unwrap();
        let mut reader = handles.reader;
        let mut buf = Vec::new();

        // Read until EOF or until we have at least 5 bytes. Terminals translate
        // \n to \r\n on output by default, so we only check for the literal
        // bytes "hello" — printf without a newline avoids that translation.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut chunk = [0u8; 64];
        loop {
            if std::time::Instant::now() >= deadline {
                panic!("timed out waiting for `hello`; got: {:?}", buf);
            }
            match reader.read(&mut chunk) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() >= 5 {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        assert!(
            buf.starts_with(b"hello"),
            "expected `hello` prefix, got {:?}",
            buf
        );
    }
}
