//! Headless demo binary: spawn one scripted child that emits OSC 1338
//! transitions, observe state changes, print them.
//!
//! Stage 4 replaces this with the winit event loop and the wgpu renderer
//! plus stdin forwarding for interactive use. For Stage 3 the demo runs
//! a non-interactive script (no stdin forwarding from the demo's own
//! stdin to the PTY child) and exits cleanly when the child terminates.

use std::time::{Duration, Instant};

use vibeflow::app::App;
use vibeflow::session::SessionEvent;

/// Default child command for the demo. Emits "starting…", a Working frame,
/// sleep 2s, a Waiting frame, sleep 2s, "done", then exits. Total runtime
/// ~5 seconds. Overridable via the `VIBEFLOW_DEMO_CMD` env var.
///
/// Octal escapes (`\033` = ESC, `\007` = BEL) are used instead of hex (`\x1b`,
/// `\x07`) because Ubuntu's `/bin/sh` is `dash`, whose `printf` interprets only
/// the octal form. The bytes produced are byte-for-byte identical.
const DEFAULT_DEMO: &str = "\
    printf 'starting...\\n'; \
    printf '\\033]1338;state=working;tool=demo\\007'; \
    sleep 2; \
    printf '\\033]1338;state=waiting;tool=demo\\007'; \
    sleep 2; \
    printf 'done\\n'";

fn main() -> std::io::Result<()> {
    eprintln!("vibeflow Stage 3 headless demo");
    let demo_cmd = std::env::var("VIBEFLOW_DEMO_CMD").unwrap_or_else(|_| DEFAULT_DEMO.into());

    let mut app = App::new();
    app.new_tab(&["/bin/sh", "-c", &demo_cmd])?;

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let mut died = false;
        for (idx, ev) in app.poll_all(Instant::now()) {
            match ev {
                SessionEvent::StateChanged(state) => {
                    eprintln!("[tab {idx}] state -> {state:?}");
                }
                SessionEvent::PassThrough(bytes) => {
                    // Stage 4+ pipes this into alacritty_terminal. For Stage 3,
                    // dump verbatim to our own stdout so the user sees shell
                    // output. Lossy on non-UTF-8 — fine for the demo.
                    let _ = std::io::Write::write_all(&mut std::io::stdout().lock(), &bytes);
                }
                SessionEvent::Died => {
                    eprintln!("[tab {idx}] died — exiting");
                    died = true;
                }
            }
        }
        if died {
            return Ok(());
        }
        for (idx, ev) in app.tick_all(Instant::now()) {
            if let SessionEvent::StateChanged(state) = ev {
                eprintln!("[tab {idx}] tick -> {state:?}");
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
