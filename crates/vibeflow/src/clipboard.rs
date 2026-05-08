//! System clipboard wrapper. Stage 8 uses CLIPBOARD only; PRIMARY is deferred.
//!
//! `Clipboard` is owned by `App` (single instance per process). On systems
//! without a display server (CI, headless containers), `Clipboard::new()`
//! returns `Err`; the caller treats this as a soft failure — vibeflow runs
//! without copy/paste in that environment, but does not crash.

use anyhow::{Context, Result};

/// Wrapper over `arboard::Clipboard`. Only exposes the operations Stage 8
/// needs: copy a `&str`, paste a `String`. Errors are logged at `warn` by
/// the caller and do not crash.
pub struct Clipboard {
    inner: arboard::Clipboard,
}

impl Clipboard {
    /// Construct a new clipboard handle. Fails on headless systems
    /// (`arboard::Error::ContextCreationFailed`) — the caller should log and
    /// continue without clipboard support.
    ///
    /// # Errors
    /// Propagates `arboard` errors connecting to the display server.
    pub fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new()
            .context("create system clipboard handle (no display server?)")?;
        Ok(Self { inner })
    }

    /// Copy `text` to the system CLIPBOARD selector.
    ///
    /// # Errors
    /// Propagates `arboard` errors. The caller logs at `warn` and proceeds —
    /// a copy failure must not crash the renderer.
    pub fn copy(&mut self, text: &str) -> Result<()> {
        self.inner
            .set_text(text)
            .context("write to system clipboard")?;
        Ok(())
    }

    /// Paste from the system CLIPBOARD selector. Returns `None` if the
    /// clipboard is empty or holds non-text content (an image, etc.).
    pub fn paste(&mut self) -> Option<String> {
        self.inner.get_text().ok()
    }

    /// Stage 9 stub. Task 10 wires PRIMARY-clipboard support.
    pub fn set_primary_enabled(&mut self, _enabled: bool) {
        // Task 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires display server (X11/Wayland) — run with --ignored"]
    fn copy_paste_roundtrips_through_system_clipboard() {
        let mut c = Clipboard::new().expect("clipboard available");
        c.copy("hello, vibeflow").unwrap();
        let got = c.paste().expect("paste returned text");
        assert_eq!(got, "hello, vibeflow");
    }
}
