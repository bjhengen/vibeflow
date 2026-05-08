//! System clipboard wrapper. Stage 8 supports CLIPBOARD; Stage 9 adds the
//! optional X11 PRIMARY selector (middle-click paste). Linux-only — silently
//! no-ops on macOS/Windows.

use anyhow::{Context, Result};

pub struct Clipboard {
    inner: arboard::Clipboard,
    /// True when the user has enabled `clipboard.primary = true` in config.
    /// On non-Linux this flag is meaningful but the underlying arboard ops
    /// fall through to CLIPBOARD anyway.
    primary_enabled: bool,
}

impl Clipboard {
    /// # Errors
    /// Propagates `arboard` errors connecting to the display server.
    pub fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new()
            .context("create system clipboard handle (no display server?)")?;
        Ok(Self {
            inner,
            primary_enabled: true, // matches default config
        })
    }

    pub fn set_primary_enabled(&mut self, enabled: bool) {
        self.primary_enabled = enabled;
    }

    pub fn primary_enabled(&self) -> bool {
        self.primary_enabled
    }

    /// Copy `text` to the CLIPBOARD selector. Also writes to PRIMARY if
    /// `primary_enabled` is true (Linux-only effect).
    ///
    /// # Errors
    /// Propagates `arboard` errors. The caller logs at `warn` and proceeds.
    pub fn copy(&mut self, text: &str) -> Result<()> {
        self.inner
            .set_text(text)
            .context("write to system clipboard")?;
        if self.primary_enabled {
            #[cfg(target_os = "linux")]
            {
                use arboard::{LinuxClipboardKind, SetExtLinux};
                let _ = self
                    .inner
                    .set()
                    .clipboard(LinuxClipboardKind::Primary)
                    .text(text);
            }
        }
        Ok(())
    }

    /// Linux-only: write `text` to the PRIMARY selector ONLY (CLIPBOARD untouched).
    /// Used by the auto-copy-on-selection-finalize path.
    ///
    /// # Errors
    /// Propagates `arboard` errors.
    pub fn copy_primary(&mut self, text: &str) -> Result<()> {
        if !self.primary_enabled {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        {
            use arboard::{LinuxClipboardKind, SetExtLinux};
            self.inner
                .set()
                .clipboard(LinuxClipboardKind::Primary)
                .text(text)
                .context("write to PRIMARY selector")?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = text; // silence unused on non-Linux
        }
        Ok(())
    }

    /// Paste from the CLIPBOARD selector. Returns `None` if empty / non-text.
    pub fn paste(&mut self) -> Option<String> {
        self.inner.get_text().ok()
    }

    /// Paste from the PRIMARY selector (X11 middle-click semantic). Returns
    /// `None` on non-Linux or if PRIMARY is empty / non-text or PRIMARY disabled.
    pub fn paste_primary(&mut self) -> Option<String> {
        if !self.primary_enabled {
            return None;
        }
        #[cfg(target_os = "linux")]
        {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            self.inner
                .get()
                .clipboard(LinuxClipboardKind::Primary)
                .text()
                .ok()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
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

    #[test]
    #[ignore = "requires X11 PRIMARY selector — Linux only"]
    fn primary_roundtrips_when_enabled() {
        let mut c = Clipboard::new().expect("clipboard available");
        c.set_primary_enabled(true);
        c.copy("primary test").unwrap();
        // Read back from PRIMARY directly.
        let got = c.paste_primary().expect("primary returned text");
        assert_eq!(got, "primary test");
    }

    #[test]
    fn primary_disabled_returns_none() {
        // Doesn't need a display server because the disabled path short-circuits.
        let Ok(mut c) = Clipboard::new() else { return };
        c.set_primary_enabled(false);
        assert_eq!(c.paste_primary(), None);
    }
}
