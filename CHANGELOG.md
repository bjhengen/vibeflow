# Changelog

All notable changes to this project are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Closing a tab can no longer freeze the window** (#30). The tab's PTY reader thread was joined unconditionally on close, but a reader parked in `read()` on the PTY master only returns once the *last* slave fd closes — and a process that inherited the slave from the tab's shell (a detached GUI app, a `nohup`'d daemon) can hold it open indefinitely, long after that shell has exited. Killing the tab's own child is not enough. Closing such a tab deadlocked the UI thread inside the click handler, which stopped the event loop, which stopped every other tab from rendering or draining its own PTY — an unrecoverable freeze of the whole window. Teardown now waits on a completion signal with a short deadline and detaches the reader if it is still blocked; a detached reader exits by itself as soon as its read returns.
- **Input to a tab whose app has stopped reading can no longer freeze the window** (#31). Every write to a PTY — keystrokes, pastes, the Ctrl+L redraw and mouse reports — was a blocking `write_all` on the UI thread. If the tab's app had mouse reporting on and had stopped draining its stdin, the 4 KiB kernel input buffer filled and the next write parked forever, taking the window with it; dragging the mouse across the window was enough to trigger it, and so was a large paste. Writes now go to a per-tab writer thread through a bounded queue, so the UI thread never blocks on a PTY. When that queue backs up, mouse-motion reports are dropped — a stale cursor position is worthless — while typed input keeps queueing.

### Internal

- Post-0.2.0 cleanups (#28): a context menu left open by a mid-drag right-click is now dismissed the moment the drag engages (was: on the first applied move); the tab-bar layout recompute idiom is collapsed into one `tab_bar_layout()` accessor; `slot_at_x`'s new-tab-button priority uses an explicit x-span check plus an overflow/underlap regression test; the `busy_tabs` foreground-subprocess test no longer races the shell's fork→exec transition.

## [0.2.0] - 2026-07-28

First feature release after the public launch. App-only — `vibeflow-protocol` stays `0.1.3` (the OSC 1338 wire format is unchanged).

### Added

- **Tabs can be reordered** (the reorder half of #9). Drag a tab along the
  bar and it snaps slot-to-slot as the cursor crosses its neighbors; a plain
  click still just switches tabs (a 4 px threshold separates the two). The
  dragged tab is highlighted and the cursor shows a grab hand. By keyboard:
  `Ctrl+Shift+PageUp` / `Ctrl+Shift+PageDown` move the active tab, rebindable
  via `[shortcuts] move_tab_left` / `move_tab_right`. (These two chords
  previously fell through to the Shift-gated scrollback half-page scroll —
  an undocumented accident of the guard; plain `Shift+PageUp`/`PageDown`
  scrollback is unchanged.) Detaching a tab into its own window is tracked
  separately (multi-window architecture).

## [0.1.7] - 2026-06-15

Pre-launch audit-hardening pass ahead of the public posts: a multi-dimension review (security, stability, performance, quality, packaging) of the whole codebase, with the confirmed findings fixed here. No known-vulnerable dependencies, all `unsafe` still forbidden, the terminal-injection surface re-verified clean. App-only — `vibeflow-protocol` stays `0.1.3` (the OSC 1338 wire format is unchanged; the only protocol-crate change is documentation).

### Security

- **OSC 52 clipboard *write* is now gateable** via `[clipboard] allow_osc52_write` (default `true`). OSC 52 write was honored unconditionally, so any terminal output — including a remote SSH session — could silently overwrite the system clipboard. It was the one untrusted-output sink not already gated (OSC titles honor `respect_osc_title`; OSC 52 *read* is always dropped). The flag is checked at the untrusted-input boundary in `poll()`, so a disabled write never reaches the clipboard. Default preserves the `vim "+y` / tmux / remote-copy workflow. Documented in `SECURITY.md` and the README, alongside a note that non-bracketed-paste newline forwarding is intentional (industry-standard; bracketed paste defends it).

### Fixed

- **Edge-drag selection no longer crashes the app.** Dragging a selection into the window's right/bottom partial-cell strip (window size is almost never an exact multiple of the cell pitch) produced a grid `Point` one past the last valid column/line; alacritty's raw `Vec` grid index panics in release, and the Linux mouse-up PRIMARY auto-copy then indexed the grid — taking down the whole process and every tab. The pixel→grid conversion now clamps to the live grid bounds at its single boundary. Regression-tested.
- **No more startup panic shortly after boot.** The redraw/activity sentinels were initialized with `Instant::now() - Duration::from_secs(3600)`, which panics on underflow; because `Instant` is boot-anchored on Linux, launching vibeflow within an hour of boot crashed before the window opened. Now a saturating subtraction that degrades to "just now" in that window (at most one missed pulse interval).
- **Tier-3 heuristic now detects interpreter-wrapped CLIs.** The `/proc` foreground match read only the process-group leader's `comm`, so a tool shipped as a `node`/`python` launcher (e.g. Codex, installed as `node /usr/bin/codex`) showed `node` — never in the AI-tools list — and got no indicator. Detection now resolves the wrapped tool name from the launcher's command line (when the leader is a known interpreter) and matches that against `[ai] tools`, so `codex`/`aider` etc. are recognized by their real name. A bare `node server.js` is still not matched unless the user lists `node`/`server` — no false positives.

### Internal

- **Panic hook for crash diagnosis.** vibeflow runs the event loop, VTE processor, and render/selection code on one thread with no per-tab isolation, so a panic unwound the process silently. A `std::panic::set_hook` now records the panic message + location through `tracing` (chained to the default hook) so it lands in the rotated file log before exit.
- **Render prefers the Fifo (vsync) present mode** instead of `present_modes[0]` (adapter-ordered, not guaranteed Fifo) — caps a mostly-static terminal at the refresh rate and avoids tearing; falls back to index 0 if Fifo is unavailable.
- **`vibeflow-protocol`'s `Frame` fields are now documented** and the crate enables `#![warn(missing_docs)]`, so CI's `-D warnings` catches any future undocumented public item in the published library.
- **The config-watcher debounce/Remove-cancel decision is unit-tested.** Extracted into a pure `decide()` function and covered for all three transitions (the Remove-cancels-pending-reload branch had a documented past bug and no test); removed an empty `#[ignore]` test that referenced a non-existent integration file.
- **Shipped Claude Code integration ships the full five-hook set.** `integrations/claude-code-hooks.json` had only two hooks — exactly the configuration the README warns causes indicator flicker on multi-tool-call turns. Now matches the README's five-hook set; `integrations/README.md` updated accordingly.
- **Dev-environment identifiers scrubbed** from tracked docs and source (absolute home path, internal hostname in PS1 examples / the finale checklist) ahead of the repo going public.

## [0.1.6] - 2026-06-14

Daily-driver hardening cycle from real use ahead of the launch posts (PRs #20–#22): a VNC/remote-X screen-flicker fix with an opt-out, PTY-firehose backpressure, and a differential fuzzer for the streaming OSC dispatcher. App-only — `vibeflow-protocol` stays `0.1.3` (the OSC 1338 protocol is unchanged this cycle).

### Added

- **`[ui] indicator_pulse` config key** (default `true`) to fix screen flicker under VNC / remote X (#19). A `Waiting` tab's amber indicator pulses via a continuous 1.4 s sine animation; each pulse frame re-renders, and with no partial/damaged present in wgpu 0.20 that forces a full-surface present every frame. On a software X server (VNC, remote X) each full present is re-encoded as full-screen damage, perceived as a lighter-grey screen flicker that worsens with the number of concurrently-`Waiting` tabs. Setting `indicator_pulse = false` renders the indicator steady (no per-frame change) and drops the Waiting-tab paint cadence back to the idle rate, eliminating the flicker. Local GPU displays are unaffected and keep the default. Hot-reloads via the existing config watcher.

### Fixed

- **PTY reader channel is now bounded (#17).** The reader thread → main-loop channel was an unbounded `mpsc::channel`; a sustained output firehose (`cat /dev/zero`, `yes`, a runaway agent dumping gigabytes) could buffer unbounded heap between polls (reader produces at hundreds of MB/s, parser drains at ~9 MB/s). It is now a `sync_channel(512)` (~2 MiB/tab): the reader blocks on a full queue, the PTY kernel buffer fills, and the child's writes block — backpressure, no bytes dropped. Teardown drops the receiver before joining the reader thread so closing a tab mid-firehose can't deadlock. Steady-state throughput is unchanged.

### Internal

- **Fuzz target for the streaming OSC dispatcher (#18).** New `crates/vibeflow/fuzz` crate with an `osc_dispatch` libfuzzer target: it feeds arbitrary input through `OscDispatcher::feed` as random segments and asserts the resulting event stream matches feeding the input whole (after coalescing `PassThrough` runs) — a differential check targeting split-frame reassembly — plus the no-panic property. Runs 60s in the CI fuzz smoke alongside the protocol `parse` fuzzer.

## [0.1.5] - 2026-06-12

Input-path hardening and repo hygiene ahead of the public launch posts (PRs #15, #16). The `vibeflow` app moves to `0.1.5`; `vibeflow-protocol` stays `0.1.3` (the OSC 1338 protocol is unchanged this cycle).

### Security

- **Tab titles are sanitised before rendering.** OSC 0/2 title payloads now strip control characters (C0/C1/DEL) and Unicode bidi formatting codepoints (overrides, embeddings, isolates, direction marks) before the existing 1024-char cap. Previously a guest program could plant RTL overrides or raw controls in the rendered tab title — visual spoofing of the tab's apparent name or state.
- **Bracketed-paste sanitisation hardened.** The paste-end marker strip now also covers the 8-bit C1 form (`U+009B` + `201~`) and loops until stable, closing a splice where removing one marker could reassemble a fresh one from the surrounding bytes (`ESC[2` + marker + `01~`).
- **OSC 52 decode allocation bounded.** The base64 payload is clipped to the largest 4-aligned prefix that decodes within the 100 KB raw cap *before* decoding (defence in depth; unreachable via the dispatcher's smaller 128 KB envelope).

### Fixed

- **`set_label` custom subtitles now stick.** A subtitle installed via `PtySession::set_label` survives the activity-driven subtitle refresh instead of being stomped on every state transition (closes the long-standing `TODO(stage9-config)`).

### Added

- **`SECURITY.md`** — private vulnerability disclosure policy — and **`CONTRIBUTING.md`** — build/test gates, PR conventions, and pointers for third-party OSC 1338 implementations.

### Internal

- Production `Term::new`/`Term::resize` sizing now uses a local `GridSize` (`grid::Dimensions` impl) instead of importing `alacritty_terminal::term::test::TermSize`.
- Two guarded `Option::unwrap()`s in `window.rs` rewritten as `let-else`; `proc_watch` doc comments corrected (`tpgid` is field 8 in proc(5) numbering — parsing logic was already right); CHANGELOG footer release links completed.

## [0.1.4] - 2026-06-01

Daily-driver fixes from real-world use (GitHub issues #6, #7, #8, #10). The `vibeflow` app moves to `0.1.4`; `vibeflow-protocol` stays `0.1.3` (the OSC 1338 protocol is unchanged this cycle).

### Added

- **Tab rename selects the whole name on entry** (#8). Starting a rename now shows the existing name highlighted with the caret suppressed, so the first keystroke or Backspace replaces it; Arrow/Home/End collapse the selection without clearing. Fixes the prior bug where a name wider than the tab rendered the edit caret off-tab (over the next tab) until you backspaced it back into view.

### Fixed

- **New tabs fill the window width immediately** (#6). New sessions spawned at the 80×24 default and weren't resized to the live window until the next resize event, so a new tab opened in a constrained column. Both the keyboard (Ctrl+Shift+T) and `+`-button paths now size the tab to the current window on creation.
- **AI-state Waiting indicator no longer vanishes after 30 s** (#7). On the Tier-3 heuristic path (Claude Code detected via `/proc`, no OSC 1338 hooks), the amber "Waiting" cue used to revert to neutral "active" after the 30 s stale-state timeout. `Waiting` is now exempt from that timeout, recovers to `Working` when output resumes, and clears when the AI tool exits. The explicit OSC 1338 path is unchanged.
- **UI stays responsive during heavy output bursts** (#10). `PtySession::poll()` drained the entire reader-channel backlog in one synchronous byte-by-byte parse, so a multi-MB burst (agent builds/diffs/file dumps) froze the main loop for hundreds of ms to seconds — no input, no repaint. `poll()` now consumes at most 64 KB per call and re-wakes immediately while a backlog remains, so output catches up at full throughput while input and repaint stay live and interruptible. `TermUpdated` is coalesced to one per poll.

## [0.1.3] - 2026-05-28

Confirm-on-close: a modal dialog now guards close paths that would discard active sessions or AI work. Two-part landing — the original spec covers window-close (`WindowEvent::CloseRequested`); a same-day amendment after the VNC smoke walk extends the gate to per-tab close paths (Ctrl+Shift+W, X-button, "Close Other Tabs"). Plus stability fixes and a long-deferred TODO closeout.

### Added

- **Confirm-on-close dialog for window close.** `WindowEvent::CloseRequested` now opens a centred modal overlay when the window has >1 tab open OR any tab is "busy" (foreground subprocess beyond the shell, OR AI tracker in `Working` / `Waiting`). Single idle tab still closes silently. iTerm2-style protective default. Cancel button is focused first so muscle-memory Enter spam can't discard in-flight work. ESC dismisses; second close-request rage-quits. Mirrors the v0.1.2 About-overlay rendering pattern (rects through `TabBarPipeline`, glyphs through `QuadPipeline` — no new render pass).
- **Per-tab close confirmation** (same-release amendment). `Shortcut::CloseTab` (Ctrl+Shift+W / Super+W), `TabBarHit::TabClose` (X-button click), and `MenuAction::CloseOtherTabs` now route through the same dialog with scope-appropriate title ("Close this tab?" / "Close other tabs?") and confirm-button label ("Close tab" / "Close other tabs"). Single-idle-tab close stays silent even in a multi-tab window — closing one idle tab is a contained, deliberate action; surviving tabs are safe. "Close Other Tabs" confirms when >1 tab would close OR any of those tabs is busy.
- **`[ui]` config section** with `confirm_on_close: bool` (default `true`). Flipping to `false` bypasses all four confirm paths (window close + the three per-tab paths) — for users who never want a dialog.
- **`PtySession::has_foreground_child`** — Linux `/proc/<pid>/stat` tpgid check. True when something other than the shell holds terminal control (`python3`, `vim`, `claude`, `make`, …). Used by busy detection and surfaced via `App::busy_tabs` for the dialog's session list.
- **`PtySession::detected_ai_tool`** — name of the most-recently-matched `tools_list` entry, captured during the existing Stage 11 proc check. Lets the dialog show "claude" / "codex" instead of the raw FG comm.
- **`proc_watch::foreground_pgid`** — companion to `foreground_command_name` that returns the tpgid directly (no extra `/proc/<tpgid>/comm` read). Used by `has_foreground_child`.

### Changed

- **`App::close_tab` last-tab behaviour: tabless window → exit.** The v0.1.0 sentinel state ("close last tab, app sits there with no tabs") was a deferred TODO at `App::close_tab` line ~203. After the per-tab amendment landed it was immediately visible during smoke walk; `WindowApp` now treats `App.tabs().is_empty()` as exit-time via a small `exit_if_no_tabs` helper called from every close-tab dispatch site.
- **Stderr default filter** raised to `vibeflow=warn` for the v0.1.2 file-logging facility (already shipped; no user-visible change in v0.1.3, restated for completeness).

### Fixed

- **6 timing-flaky lib + integration tests** under default parallel `cargo test` load. The pattern `send_input("sleep 30\n") + thread::sleep(500ms) + assert_busy` wasn't long enough for bash to fork sleep with ~50 concurrent PtySession tests competing for CPU. Replaced with a `wait_until(timeout, cond)` poll-with-deadline helper (mirrors the existing `app::tests` polling pattern at lines 477+). 3× consecutive parallel runs now pass with zero failures.
- **`tier_3_arms_on_rising_edge_even_without_real_output`** (pre-existing Stage 11 test). Was synthetic-time previously; switching to real-time polling exposed a latent flake: `/bin/sh -c "sleep 5"` `exec()`s into `sleep` mid-test, so a single-name `tools_list = ["sh"]` un-arms the heuristic on the silence-check tick after the comm flips from `"sh"` to `"sleep"`. Fix: arm with a candidate set `{sh, bash, dash, sleep}` so the heuristic stays armed regardless of exec timing or which `sh` implementation is at `/bin/sh`.

### Internal

- **`busy_info_for(&PtySession, idx)`** extracted from inline `busy_tabs` logic; now shared by `App::busy_tabs`, `App::tab_busy_info`, and `App::tabs_busy_except`. Same predicate, three call sites.
- **`ConfirmCloseScope { Window, SingleTab(idx), OtherTabs(keep_idx) }`** drives per-scope title text and confirm-button label. Constructor `ConfirmCloseState::new(busy, tab_count)` keeps existing call sites + tests working (defaults `scope = Window`); new `with_scope` covers the per-tab paths.
- **`dispatch_confirm_close_confirm`** centralises the confirm action across keyboard Enter + LMB Pressed on the destructive button. Match on scope: `Window → pending_exit`, `SingleTab(idx) → close_tab + redraw`, `OtherTabs(k) → loop close + set_active(0) + redraw`.

## [0.1.2] - 2026-05-25

The first feature-bundle release after v0.1. Five sub-features merged via individual PRs (#1–#4 plus the direct-merged logging facility), each with senior pre-execution review of plans vs actual source, subagent-driven per-task implementation with two-stage review, and a manual VNC smoke walk before merge.

### Added

- **CLI `--version` / `-V` flag.** Short-circuits before any winit/wgpu init so the version is queryable headless (over SSH, in Dockerfile builds, in CI). 3 integration tests pin exit code 0, exact stdout `vibeflow {version}\n`, empty stderr, and elapsed-time bound (no GUI fall-through).
- **"About vibeflow" right-click menu item → centred modal overlay panel.** 5 lines (version, gap, tagline, license + repo URL, dismissal hint). 880×240 logical pixels, theme-coloured, dim backdrop. ESC / any keypress / any mouse click closes (and swallows the event so it doesn't leak to the PTY).
- **File logging facility** (Spec A', closes the Stage-9 TODO). Default stderr filter is now `vibeflow=warn` (quiet by default); INFO+ is captured to a daily-rotated file at `$XDG_STATE_HOME/vibeflow/vibeflow.log.YYYY-MM-DD`. `RUST_LOG` env var still overrides at runtime.
- **OSC 52 clipboard WRITE** (Codex review #5 resolved). Full pipeline parser → `DispatchEvent::Osc52Write` → `SessionEvent::Osc52ClipboardWrite` → `Clipboard::copy_clipboard_only` / `copy_primary`. Read is intentionally not implemented (security default — matches xterm/foot/wezterm — see threat model in the OSC 52 design spec).
- **Per-flag terminal attribute rendering.** `BOLD` and `ITALIC` cells route to embedded JetBrains Mono variant fonts via a `font_attrs_for(cell.flags)` helper; `UNDERLINE` / `DOUBLE_UNDERLINE` / `UNDERCURL` / `DOTTED_UNDERLINE` / `DASHED_UNDERLINE` / `STRIKEOUT` render as decoration quads; `INVERSE` / `HIDDEN` / `DIM` mutate per-cell fg/bg before glyph emission. Order of operations within the cell loop is `INVERSE → HIDDEN → DIM → cursor-invert`. The atlas cache key gains `(Weight, Style)` so all four font variants share the same atlas without collision.
- **JetBrains Mono Bold + Italic + BoldItalic v2.304 fonts embedded** alongside the existing Regular. AppImage growth ~+1.8 MB.
- **Glyph atlas hard caps** with full-reset on overflow (4096 px mono, 2048 px color). Adversarial-input safety net — `tracing::warn!` on reset; cache.clear() + shelves.clear() + texture recreate at initial size; next paint re-rasterises naturally.
- **Supply-chain hardening** (Spec B): all `release.yml` and `ci.yml` GitHub Actions are SHA-pinned to specific commits; `actions/attest-build-provenance@v4.1.0` generates an SLSA build provenance attestation alongside the AppImage; SBOM (cyclonedx) is generated and uploaded with the release; new packaging-assertions CI job validates `.crate` + npm-pack contents end-to-end AND enforces Cargo workspace version == `bindings/npm/package.json` version (catches the v0.1.1 npm-version slip pattern).
- **Config-value bounds** (Codex review #8): `bounds::clamp_with_warn` clamps `ai.polling_interval_ms`, `scrollback.history_lines`, etc. with a warn-log on out-of-range values rather than silently accepting them.

### Changed

- **Cursor rendering for empty cells** (the headline Claude Code TUI bug). The cursor cell is iterated by alacritty's `display_iter` (the earlier hypothesis that empty cells are skipped was wrong); the actual bug was an over-aggressive `continue` that suppressed `INVERSE`-flagged "visual cursor" characters that TUI libraries like Ink draw at the cursor position after hiding the terminal cursor via DECTCEM `?25l`. Cell content now renders regardless of terminal-cursor visibility state; the per-cell `INVERSE` handler above correctly inverts Ink's visual cursor; a standalone cursor quad is emitted after the cell loop only when the cursor cell wasn't iterated at all.
- **Paint cadence — idle CPU 78% → 5%** (the headline performance bug). `about_to_wait` previously called `request_redraw()` unconditionally on every wake; winit on X11 (especially over VNC) wakes `about_to_wait` from background X11 events far more often than vsync rate, which drove vibeflow to paint at ~60 FPS continuously even on a bare bash prompt. The new logic gates `request_redraw()` behind an elapsed-time deadline derived from cursor-blink boundary (idle) or pulse interval (`TabState::Waiting`). Most wakes now just re-set `WaitUntil` and return.
- **Typing latency close to xterm.** New `last_activity_at` tracks user keypresses + PTY echo events; while activity is recent (within 500 ms), `WaitUntil` tightens to 4 ms so the PTY echo queue is drained at ~vsync latency, then falls back to the blink boundary for low idle CPU.
- **About-overlay default panel size 560×200 → 880×240.** First-VNC-smoke catch: the spec's 560 px width didn't fit the tagline or license + URL lines at JetBrains Mono's actual cell pitch (`lesson_layout_default_too_small`).
- **Default startup is quieter.** Per the logging change, stderr no longer streams `info!` from winit / wgpu / cosmic-text / vibeflow's own per-event logs; only `warn!`+`error!` reach stderr at the new default filter.
- **Theme registry capped at 50 themes loaded** (Codex review #8, defensive against `~/.config/vibeflow/themes/` directories with many `.toml` files).
- **`--import-colors` rejects files exceeding 256 KB** (Codex review #8, defensive against pathological iTerm2 `.itermcolors` payloads).
- **`MenuAction::OpenRepoUrl` removed.** The grid-menu "About vibeflow" item used to spawn `xdg-open <REPO_URL>`; with the new About overlay showing the URL as visible text, the `xdg-open` path is no longer needed.

### Fixed

- **Decoration quads (UNDERLINE / STRIKEOUT / etc.) render in solid fg color** rather than blending with whatever pixel happened to live at atlas (0, 0). The shader's mono path is `mix(bg, fg, alpha)`; with a zero-size atlas rect, `alpha` is sampled at atlas (0, 0) — an undefined first-glyph pixel. Passing `(fg, fg)` instead of `(fg, bg)` resolves to pure `fg` regardless. Pinned by a regression test in `quad.rs::tests`.

### Internal

- **Final v0.1.2 sub-feature merged via PR #4** (Render). v0.1.2 shipped via four GitHub PRs (#1 OSC 52, #2 supply-chain, #3 About, #4 Render) plus one direct merge (logging). The PR-based workflow was adopted mid-v0.1.2 and is now the standing pattern for non-trivial work (`feedback_pr_workflow`).
- **Subagent-driven-development cumulative scale**: across the 4 PRs, ~40 implementation tasks were dispatched to fresh per-task subagents with two-stage review (spec-compliance then code-quality). Senior pre-execution Sonnet review of each plan vs actual source caught 12+ compile blockers before dispatch.

## [0.1.1] - 2026-05-20

### Changed

- **Default palette readability** (slots 4 and 12): the classic xterm blue `#0000ee` (slot 4) is too dark to read against typical dark terminal backgrounds — `ls --color=auto` uses `01;34` for directories, which renders as bold + slot 4 in most shells, producing barely-readable folder names. Slot 4 now defaults to `#6a76fb` (alacritty's value; passes WCAG AA against `#0e0e12`); slot 12 (bright blue) is bumped from `#5c5cff` to `#89b4fa` (Catppuccin Mocha) to keep bright-blue distinctly brighter than blue and clear WCAG AAA. Imported iTerm2 themes override these as before.

### Fixed

- **npm packaging from a clean checkout** (Codex review #1): `bindings/npm/package.json` lacked `prepack` / `prepublishOnly` scripts, so a publish without a pre-existing local `dist/` would ship an empty package. v0.1.0 was healthy by accident (our publish machine had `dist/` from `npm test`); v0.1.1 onward is correct unconditionally.
- **LICENSE files included in published Cargo and npm packages** (Codex review #3): both crates declared `MIT OR Apache-2.0` but the LICENSE text was not in the published artifacts. Now symlinked into each crate dir (Cargo materialises symlinks during packaging) and copied into `bindings/npm/` (npm pack does NOT follow symlinks; uses file copies).
- **README OSC 52 claim removed** (Codex review #5): the README claimed clipboard via OSC 52, but `session/osc.rs` only handles OSC 0/1/2/133/1338. The claim is dropped (arboard support remains). Actual OSC 52 implementation deferred — has real permission/security design work.
- **Clipboard paste sanitised against `ESC[201~` injection** (Codex review #4): bracketed paste mode wraps content with `ESC[200~`/`ESC[201~`. If the clipboard text itself contains `ESC[201~`, a paste could terminate the bracketed-paste frame mid-content and inject the remainder as live user input. Now stripped before the inner bytes reach the PTY.

### Security

- **`appimagetool` SHA256 verification in release CI** (Codex review #2, narrowed): the release workflow now verifies the pinned `appimagetool 1.9.1` against `ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0` before making it executable. The job also emits `vibeflow-x86_64.AppImage.sha256` alongside the AppImage in the GitHub Release for downstream verification.
- **`cargo doc` clean under `-D warnings`**: a broken intra-doc link in `app.rs` had been failing CI's `cargo doc` step on every push since the post-Stage-13 polish merge. Resolved (already on main as commit `387fde2`, before this branch — documented here as part of the v0.1.0→v0.1.1 delta).

### Internal

- **CI packaging-assertion job** (Codex review #10): new job runs `cargo package --list`, `cargo publish --dry-run`, and `npm pack --dry-run` and asserts the expected file set including LICENSE files. Catches future regressions of the #1 / #3 gaps automatically.
- `release.yml` gains `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` to silence the Node 20 deprecation warning, matching `ci.yml`.

## [0.1.0] - 2026-05-19

Initial public release of the vibeflow terminal — a from-scratch GPU-accelerated Linux terminal emulator whose flagship feature is **per-tab AI-state awareness** via the OSC 1338 protocol.

### Added

- **Per-tab AI-state indicator** (the flagship). A thin stripe per tab visualises whether the program inside is *working* (blue), *waiting on you* (amber, pulsing), or neutral. State arrives via the OSC 1338 protocol; three-tier resolution: native OSC 1338 (Tier 1), `/proc` heuristic on a configured AI-tools list (Tier 3). Tier 2 wrapper shims are planned post-v0.1.
- **OSC 1338 protocol** — open standard for AI-tool state signalling in terminals. Rust reference implementation as the `vibeflow-protocol` crate (includes the `vibeflow-emit` CLI used by tool hooks); TypeScript binding as the `vibeflow-protocol` npm package.
- **GPU rendering** via wgpu — fast text rendering with truecolor, italics, and color emoji.
- **Tabs** with per-tab session, title, indicator, context menu, drag-to-rename.
- **Themes** — full iTerm2 `.itermcolors` import (`vibeflow --import-colors <path>`); per-tab override; theme registry; hot-reload via `[colors] preset` config key.
- **Configurable bell** — `visual` / `audible` / `both` / `silent`, with debounce.
- **Selection** — character, word, line; **block (Alt+drag) column selection**; shift-extend; `arboard` clipboard.
- **Scrollback** — scrollable buffer with `snap_on_esc`; `Shift+PgUp` / `Shift+PgDn` navigation; correct selection mapping across scrollback rows.
- **Keyboard** — xterm-style modifier arrow keys (`Ctrl+`/`Shift+` arrows), Unicode input, configurable bindings.
- **Configuration** — `~/.config/vibeflow/config.toml` with hot-reload; sections for fonts, colors, bell, AI, scrollback, keybindings.
- **Window icon** — the vibeflow logo, embedded at compile time; shown in the launcher/taskbar entry (also in the AppImage).
- **Distribution** — `cargo install vibeflow`, and a single-file `vibeflow-x86_64.AppImage` attached to the GitHub Release.

### Out of scope for v0.1

- Splits/panes; in-buffer search; macOS/Windows builds; image protocols (kitty/sixel); plugin layer; telemetry; Python binding; headless GPU snapshot tests; binary signing/notarization; `.deb`/Homebrew/AUR packaging.

[Unreleased]: https://github.com/bjhengen/vibeflow/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/bjhengen/vibeflow/releases/tag/v0.2.0
[0.1.7]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.7
[0.1.6]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.6
[0.1.0]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.0
[0.1.1]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.1
[0.1.2]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.2
[0.1.3]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.3
[0.1.4]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.4
[0.1.5]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.5
