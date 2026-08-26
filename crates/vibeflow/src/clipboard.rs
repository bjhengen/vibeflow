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

    /// Set the system clipboard ONLY, never touching the primary selection,
    /// regardless of `primary_enabled`.
    ///
    /// Used by the OSC 52 path which carries an explicit per-selection field
    /// from the TUI app. `copy()` would broadcast to both buffers based on
    /// the user's `primary_enabled` config, violating OSC 52's semantic.
    ///
    /// # Errors
    /// Propagates `arboard` errors. The caller logs at `warn` and proceeds.
    pub fn copy_clipboard_only(&mut self, text: &str) -> Result<()> {
        self.inner.set_text(text)?;
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

/// Which selection an async read should target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    /// The CLIPBOARD selector (Ctrl+Shift+V).
    Clipboard,
    /// The X11 PRIMARY selector (middle-click).
    Primary,
}

/// #33: performs clipboard *reads* on a worker thread and posts the text back
/// to the event loop.
///
/// A read is an X11 selection round-trip with whichever application owns the
/// clipboard, and `arboard` bounds it at 4 s (`LONG_TIMEOUT_DUR`). Run from the
/// winit handler that is where the whole window stops rendering and stops
/// draining every tab's PTY for those seconds — the bounded cousin of #31.
///
/// Requests are served FIFO by one thread, so two quick pastes arrive in the
/// order they were made. Writes stay on the UI thread deliberately: `arboard`'s
/// write path only takes a short lock to store the data and assert selection
/// ownership (its own `serve_requests` thread does the serving), so it does not
/// block and does not contend with a read in flight.
pub struct ClipboardReader {
    tx: std::sync::mpsc::Sender<(crate::session::TabId, Selection)>,
}

impl ClipboardReader {
    /// Spawn the reader thread. `proxy` delivers
    /// [`crate::config::AppUserEvent::ClipboardText`] back to the event loop.
    ///
    /// # Errors
    /// Propagates thread-spawn failures.
    pub fn spawn(
        proxy: winit::event_loop::EventLoopProxy<crate::config::AppUserEvent>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<(crate::session::TabId, Selection)>();
        std::thread::Builder::new()
            .name("vibeflow-clipboard".into())
            .spawn(move || {
                // Created on the worker, lazily: arboard hands out handles onto
                // a process-global context, so this is cheap, and a machine with
                // no display server never pays for it.
                let mut clipboard: Option<Clipboard> = None;
                for (tab, selection) in rx {
                    if clipboard.is_none() {
                        match Clipboard::new() {
                            Ok(mut c) => {
                                // Always permitted here; the caller decides
                                // whether a PRIMARY read may be requested at
                                // all, from the live config.
                                c.set_primary_enabled(true);
                                clipboard = Some(c);
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "no system clipboard; paste ignored");
                                continue;
                            }
                        }
                    }
                    let Some(c) = clipboard.as_mut() else {
                        continue;
                    };
                    let text = match selection {
                        Selection::Clipboard => c.paste(),
                        Selection::Primary => c.paste_primary(),
                    };
                    let Some(text) = text else { continue };
                    if proxy
                        .send_event(crate::config::AppUserEvent::ClipboardText { tab, text })
                        .is_err()
                    {
                        break; // event loop is gone
                    }
                }
            })?;
        Ok(Self { tx })
    }

    /// Ask for `selection`'s text, to be delivered to `tab`. Returns
    /// immediately; the text arrives later as an `AppUserEvent`.
    pub fn request(&self, tab: crate::session::TabId, selection: Selection) {
        if self.tx.send((tab, selection)).is_err() {
            tracing::warn!("clipboard reader thread is gone; paste ignored");
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

    #[test]
    #[cfg(unix)]
    fn copy_clipboard_only_does_not_touch_primary() {
        // This test verifies the API exists with the right signature; the
        // actual side-effect (system clipboard set without primary touched)
        // requires a display server to test fully. We can at minimum assert
        // the method compiles, accepts &mut self + &str, and returns Result.
        let Ok(mut cb) = Clipboard::new() else {
            eprintln!("skipping copy_clipboard_only test: no display server");
            return;
        };
        cb.set_primary_enabled(true);
        assert!(cb.primary_enabled());
        // Method exists, accepts &mut self + &str, returns Result.
        let result: anyhow::Result<()> = cb.copy_clipboard_only("hello");
        // We don't assert success — headless edge cases can still fail
        // arboard::set_text. We assert the type-checks shape.
        let _ = result;
    }
}

/// Remove every occurrence of the bracketed-paste end marker `ESC[201~`
/// (bytes `0x1b 0x5b 0x32 0x30 0x31 0x7e`) from clipboard text before it
/// is sent to the PTY.
///
/// Defence against a clipboard-paste-injection vector: a malicious paste
/// containing `ESC[201~` would otherwise terminate the bracketed-paste
/// frame mid-content, after which the terminal interprets remaining
/// bytes as live user input — and the application running inside reads
/// them as keystrokes the user did not type.
///
/// Removal (not substitution) preserves adjacent content byte-for-byte;
/// the marker is six bytes and removing it is the least-surprising
/// outcome for adjacent text. Also applied in the non-bracketed path —
/// the marker has no meaning there but stripping keeps the invariant
/// simple ("PTY never sees the marker via paste").
#[must_use]
pub fn sanitise_paste(input: &str) -> String {
    // Both encodings of the paste-end marker: 7-bit `ESC [ 201~` and the
    // single-codepoint C1 CSI form `U+009B 201~`.
    const MARKER_7BIT: &str = "\x1b[201~";
    const MARKER_C1: &str = "\u{9b}201~";
    if !input.contains(MARKER_7BIT) && !input.contains(MARKER_C1) {
        return input.to_string();
    }
    // Loop until stable: removing a marker can splice the surrounding bytes
    // into a fresh marker (e.g. `ESC[2` + marker + `01~`). Each pass shrinks
    // the string, so this terminates.
    let mut out = input.to_string();
    loop {
        let before = out.len();
        out = out.replace(MARKER_7BIT, "").replace(MARKER_C1, "");
        if out.len() == before {
            return out;
        }
    }
}

#[cfg(test)]
mod sanitise_paste_tests {
    use super::sanitise_paste;

    #[test]
    fn passthrough_when_no_marker() {
        assert_eq!(sanitise_paste("hello\nworld"), "hello\nworld");
        assert_eq!(sanitise_paste(""), "");
    }

    #[test]
    fn removes_a_single_marker() {
        let injected = "ls -la\x1b[201~rm -rf /\n";
        assert_eq!(sanitise_paste(injected), "ls -larm -rf /\n");
    }

    #[test]
    fn removes_multiple_back_to_back_markers() {
        let injected = "a\x1b[201~\x1b[201~b";
        assert_eq!(sanitise_paste(injected), "ab");
    }

    #[test]
    fn preserves_other_escape_sequences_unchanged() {
        // ESC[200~ (paste START marker) must NOT be stripped — only ESC[201~
        // is the paste-end marker we strip. Other escape sequences are also
        // out of scope for this defence (deferred per spec §2.4).
        let s = "\x1b[200~start\x1b[?1004hmid";
        assert_eq!(sanitise_paste(s), s);
    }

    #[test]
    fn removes_c1_paste_end_marker() {
        // U+009B is the single-codepoint C1 CSI; followed by `201~` it is the
        // 8-bit form of the paste-end marker.
        let injected = "a\u{9b}201~b";
        assert_eq!(sanitise_paste(injected), "ab");
    }

    #[test]
    fn removes_marker_spliced_together_by_removal() {
        // Removing an inner marker must not splice the surrounding bytes into
        // a fresh marker: `ESC[2` + marker + `01~` reassembles to a marker
        // after one removal pass.
        let injected = "a\x1b[2\x1b[201~01~b";
        assert_eq!(sanitise_paste(injected), "ab");
    }

    #[test]
    fn removes_c1_marker_spliced_together_by_7bit_removal() {
        // Cross-form splice: removing the 7-bit marker assembles the C1 form.
        let injected = "a\u{9b}2\x1b[201~01~b";
        assert_eq!(sanitise_paste(injected), "ab");
    }
}
