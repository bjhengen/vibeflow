//! Stage 13 integration tests — theme registry + per-session application
//! against a real PTY and real filesystem. Per amendment A1, assertions
//! are on `session.theme_colors()` (the resolved table the renderer
//! consults) — `Term` is never mutated by the theme system.

use alacritty_terminal::vte::ansi::NamedColor;
use std::time::{Duration, Instant};
use vibeflow::app::App;
use vibeflow::theme::registry::ThemeRegistry;
use vibeflow::theme::ThemeData;

fn drive_until(app: &mut App, deadline: Instant) {
    while Instant::now() < deadline {
        let now = Instant::now();
        let _ = app.poll_all(now);
        let _ = app.tick_all(now);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn make_theme(name: &str, fg: [f32; 4], bg: [f32; 4]) -> ThemeData {
    ThemeData {
        name: name.into(),
        ansi: [[0.5; 4]; 16],
        foreground: fg,
        background: bg,
        cursor: [0.5, 0.5, 0.5, 1.0],
        cursor_text: [0.0, 0.0, 0.0, 1.0],
        bold: None,
        link: None,
        selection: None,
    }
}

#[test]
fn theme_set_populates_session_colors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let theme = make_theme("test_theme", [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
    std::fs::write(tmp.path().join("test_theme.toml"), theme.to_toml()).expect("write");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf());
    assert!(registry.get("test_theme").is_some());

    let mut app = App::new();
    let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(200));
    let active = app.active();
    app.tabs_mut()[active].set_theme(Some("test_theme".into()), &registry);

    // A1: assert on session.theme_colors() (NOT term().colors()).
    let s = &app.tabs()[active];
    assert_eq!(s.theme(), Some("test_theme"));
    let colors = s.theme_colors().expect("theme_colors populated after hit");
    let fg = colors[NamedColor::Foreground].expect("foreground slot set");
    assert_eq!(fg.r, 255);
    assert_eq!(fg.g, 0);
    assert_eq!(fg.b, 0);
    // ANSI loop regression guard: every ansi slot is 0.5 -> 127.
    assert_eq!(colors[NamedColor::Red].expect("ansi red slot").r, 127);
    assert_eq!(
        colors[NamedColor::BrightWhite].expect("ansi 15 slot").b,
        127
    );
    // Background was [0,0,1,1] -> blue.
    let bg = colors[NamedColor::Background].expect("background slot");
    assert_eq!(bg.r, 0);
    assert_eq!(bg.b, 255);
}

#[test]
fn theme_per_tab_isolation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = make_theme("alpha", [1.0, 0.0, 0.0, 1.0], [0.0; 4]);
    let b = make_theme("beta", [0.0, 1.0, 0.0, 1.0], [0.0; 4]);
    std::fs::write(tmp.path().join("alpha.toml"), a.to_toml()).expect("write");
    std::fs::write(tmp.path().join("beta.toml"), b.to_toml()).expect("write");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf());

    let mut app = App::new();
    let _ = app
        .new_tab(&["/bin/sh", "-c", "sleep 5"])
        .expect("spawn tab0");
    let _ = app
        .new_tab(&["/bin/sh", "-c", "sleep 5"])
        .expect("spawn tab1");
    drive_until(&mut app, Instant::now() + Duration::from_millis(300));

    app.tabs_mut()[0].set_theme(Some("alpha".into()), &registry);
    app.tabs_mut()[1].set_theme(Some("beta".into()), &registry);

    let t0 = app.tabs()[0].theme_colors().expect("tab0 colors");
    let t1 = app.tabs()[1].theme_colors().expect("tab1 colors");
    assert_eq!(t0[NamedColor::Foreground].unwrap().r, 255); // alpha red fg
    assert_eq!(t1[NamedColor::Foreground].unwrap().g, 255); // beta green fg
                                                            // distinct
    assert_ne!(
        t0[NamedColor::Foreground].unwrap(),
        t1[NamedColor::Foreground].unwrap()
    );
}

#[test]
fn missing_theme_retains_name_reverts_colors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf()); // empty

    let mut app = App::new();
    let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(200));
    let active = app.active();
    app.tabs_mut()[active].set_theme(Some("ghost".into()), &registry);

    let s = &app.tabs()[active];
    // Per the T14 reconciliation: requested name retained as intent...
    assert_eq!(s.theme(), Some("ghost"));
    // ...but colors reverted to None (default palette) since unresolved.
    assert!(s.theme_colors().is_none());
}

/// Stage 13 FN1 (App-layer portion): new tabs inherit the app default theme
/// name; a per-tab override survives a subsequent `set_default_theme` call.
///
/// FN1 (from T15): this covers the App-layer contract — new tabs inherit
/// `set_default_theme`, and a per-tab override is not clobbered by a later
/// default change. NOT covered automatically: `App::restart_active` setting
/// `s.theme = self.default_theme` (app.rs ~294) and WindowApp's RestartTab
/// handler re-resolving `theme_colors` — that restart path needs WindowApp
/// (unreachable here) and has no app.rs unit assertion on the `theme`
/// field, so it is manual-smoke-only (see the Stage 13 smoke walk).
#[test]
fn new_tab_inherits_default_theme_and_override_survives() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let alpha = make_theme("alpha", [1.0, 0.0, 0.0, 1.0], [0.0; 4]);
    let beta = make_theme("beta", [0.0, 1.0, 0.0, 1.0], [0.0; 4]);
    std::fs::write(tmp.path().join("alpha.toml"), alpha.to_toml()).expect("write");
    std::fs::write(tmp.path().join("beta.toml"), beta.to_toml()).expect("write");
    let registry = ThemeRegistry::load(tmp.path().to_path_buf());

    let mut app = App::new();
    // Set app-level default before spawning tabs.
    app.set_default_theme(Some("alpha".into()));

    let i0 = app
        .new_tab(&["/bin/sh", "-c", "sleep 5"])
        .expect("spawn tab0");
    // New tab should inherit the default theme name (App-layer contract).
    assert_eq!(app.tabs()[i0].theme(), Some("alpha"));

    // Apply the registry so theme_colors is populated for tab0.
    app.tabs_mut()[i0].set_theme(Some("alpha".into()), &registry);
    assert!(
        app.tabs()[i0].theme_colors().is_some(),
        "alpha theme should resolve"
    );

    // Give tab0 a per-tab override.
    app.tabs_mut()[i0].set_theme(Some("beta".into()), &registry);
    assert_eq!(app.tabs()[i0].theme(), Some("beta"));

    // Change app default — tab0's per-tab override must NOT be affected.
    app.set_default_theme(Some("alpha".into()));
    assert_eq!(
        app.tabs()[i0].theme(),
        Some("beta"),
        "existing tab override should survive set_default_theme"
    );

    // New tab spawned after the second set_default_theme should inherit "alpha".
    let i1 = app
        .new_tab(&["/bin/sh", "-c", "sleep 5"])
        .expect("spawn tab1");
    assert_eq!(
        app.tabs()[i1].theme(),
        Some("alpha"),
        "new tab should inherit current app default"
    );
}
