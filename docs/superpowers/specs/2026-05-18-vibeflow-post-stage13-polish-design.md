# vibeflow post-Stage-13 polish — design spec

**Date:** 2026-05-18
**Status:** Approved (brainstorm), pending implementation plan
**Branch base:** `main` @ `c052fbc` (post-Stage-13 + the `fix-vibeflow-emit-tty` AI-state bundle)

Three small, independent fixes surfaced by the Stage-13 VNC smoke walk and the
subsequent AI-state investigation. None is a regression; all are pre-existing
latent issues. Tier-2 wrapper shims are explicitly **out of scope** (deferred a
few days for hook-config testing with claude).

> Code references (file:line) below were accurate at spec time on `main`
> @ `c052fbc`; verify against current source before editing.

---

## 1. Theme-driven clear color

### Problem
`render/mod.rs` defines `const CLEAR_COLOR: wgpu::Color` = hardcoded `#0e0e12`
(~line 61), applied via `LoadOp::Clear(CLEAR_COLOR)` at the render-pass setup
(~line 560) over the whole framebuffer before cells/tab-bar draw.
`pixels_to_grid` (window.rs ~23) uses integer division, so a remainder of
`width_px % cell_w` (right edge) and `height_px % cell_h` (bottom edge) is
never covered by any cell rect and shows the raw clear color. With the default
theme (`bg ≈ #0e0e12`) this is invisible; under a custom imported theme whose
background differs, it is a visible mismatched strip.

### Design
Derive the per-frame clear color from the **active session's resolved theme
background**, falling back to `CLEAR_COLOR` when there is no theme.

- The renderer already computes `active_theme_colors: Option<alacritty_terminal::term::color::Colors>`
  for the active tab (T14, threaded into `build_cell_instances`). Reuse that
  exact value — no new plumbing.
- At the `LoadOp::Clear` site, compute:
  `let clear = active_theme_colors
       .and_then(|c| c[NamedColor::Background])
       .map(|rgb| wgpu::Color { r: rgb.r as f64/255.0, g: rgb.g as f64/255.0, b: rgb.b as f64/255.0, a: 1.0 })
       .unwrap_or(CLEAR_COLOR);`
  and use `LoadOp::Clear(clear)`.
- `CLEAR_COLOR` stays as the named no-theme fallback constant.

### Constraints / invariants
- Read-only use of the existing `theme_colors`; the A1/A2 Stage-13 invariant
  (`Term` never mutated; no `NamedColor::Bold`/`CursorText`) is preserved.
- Single-file change in `render/mod.rs`. No change to `pixels_to_grid` or the
  grid math (the remainder strip is *intentionally* not cell-covered; we only
  make the uncovered pixels match the theme).

### Tests
- Unit: a helper that maps `Option<&Colors>` → `wgpu::Color` returns the theme
  bg when `Background` is `Some`, and `CLEAR_COLOR` when `None`/no theme.
  (Pure function, testable without a GPU.)

---

## 2. Startup grid-size race

### Problem
`resumed()` (window.rs ~959-981) sizes the first PTY/grid from
`renderer.surface_size()`. On some compositors / under VNC, the surface still
reports the *requested* `960×600` (`.with_inner_size`, ~line 927) at `resumed`
time, before the real (often larger) window size is finalized. Until the first
`WindowEvent::Resized`, the grid is genuinely too small (the PTY/`Term` believe
they are ~the requested size), so a large region is uncovered. After §1 that
region is theme-blended rather than jarring, but the grid is still mis-sized
until the user (or compositor) triggers a resize.

### Design
Make initial sizing self-correcting instead of trusting one early read:

- Keep the existing `resumed()` best-effort resize unchanged (first guess).
- Add a **one-shot reconcile on the first `WindowEvent::RedrawRequested`**:
  re-query `window.inner_size()`, compute `(rows, cols)` via the *same*
  `pixels_to_grid` (reserving the tab-bar strip via `tab_bar_height_px`
  exactly as `resumed`/`Resized` do), and call `app.resize_all(rows, cols)`
  — the identical path the `WindowEvent::Resized` handler (window.rs
  ~1026-1042) uses. `resize_all` to the already-correct dimensions is a
  harmless no-op, so an explicit "differs from last applied size" comparison
  is optional (implementer may add it to avoid a redundant PTY ioctl, but it
  is not required for correctness).
- A `bool` field on `WindowApp` (e.g. `initial_size_reconciled`, init `false`)
  guards it to fire exactly once.
- The existing `Resized` handler continues to cover all later changes,
  unchanged.

### Rejected alternative
`request_inner_size()` round-trips / synthesizing a fake `Resized` — more
fragile across compositors than a first-frame reconcile, and the redraw path
is guaranteed to fire before the user sees a stable frame.

### Constraints / invariants
- Reuse the existing resize code path (extract a small private helper if it
  reduces duplication between `resumed`, the new first-redraw reconcile, and
  `Resized` — implementer's judgement; do not restructure beyond that).
- No polling, no timers. Exactly one extra resize at most, on the first frame.

### Tests
- Unit: `pixels_to_grid` already testable; add coverage that the reconcile
  decision (`inner_size` ≠ grid basis ⇒ resize) and the one-shot guard
  (fires once, not again) behave correctly. Full window-lifecycle is covered
  by the manual VNC smoke walk.

---

## 3. Unified explicit-stale recovery (one knob)

### Problem
Stage-13 Q1 made `AiStateTracker::tick()` fully inert once a session has seen
any explicit OSC 1338 frame (`explicit_seen == true`): no heuristic-silence,
no stale-state, no `OutputObserved`→Working promotion. Consequences:
1. If a self-reporting tool dies mid-state (hook crash, SIGKILL, misconfigured
   `Stop`), the tab is pinned at the last explicit state until `restart()`.
2. A formerly-AI tab continuing as a plain interactive shell (tool exited, no
   restart) never regains Tier-3 / heuristic state tracking.

### Design — one mechanism, one config knob
- `AiStateTracker` gains `last_explicit_at: Option<Instant>`, set (to `now`)
  in the `on_input` `TrackerInput::AiFrame(frame)` arm alongside
  `self.explicit_seen = true`. Any subsequent explicit frame refreshes it.
- `TrackerConfig` gains `explicit_stale_state: Duration`.
- Config: `[ai] explicit_stale_state_s` — schema `Option<u64>` → resolved
  `Ai.explicit_stale_state: Duration`. **Default 300 s** (5 min — comfortably
  longer than any realistic inter-signal gap, incl. PreToolUse/PostToolUse
  cadence from the 5-hook config). **`0` disables** the fuse (tracker keeps
  Q1's "explicit = authoritative forever" behavior).
- In `tick(now)`, **before** the existing `if self.explicit_seen { return false; }`
  early-return, add:
  ```
  if self.explicit_seen
      && self.config.explicit_stale_state > Duration::ZERO
  {
      if let Some(last) = self.last_explicit_at {
          if now.saturating_duration_since(last) >= self.config.explicit_stale_state {
              self.explicit_seen = false;
              self.last_explicit_at = None;
              if self.state == TabState::Working {
                  self.state = TabState::Active;
                  self.last_event_at = Some(now);
                  return true;
              }
              // else: fall through with explicit_seen now false — Tier-3
              // (heuristic + prompt markers) resumes for the de-escalated
              // session. A stuck Waiting keeps its amber cue until the next
              // shell/prompt activity moves it.
          }
      }
  }
  ```
  (Exact structure adjusted to the real `tick()` shape during implementation;
  the semantics above are normative.)

### Semantics (normative)
- **Stuck Working** (dead hook): after `explicit_stale_state` with no new OSC
  frame → reset to **Active**. Self-heals the real failure mode.
- **Stuck Waiting**: the amber "needs you" headline cue **persists** (per the
  brainstorm decision — its whole value is persisting until the user acts). On
  the fuse, the Waiting branch clears `explicit_seen` AND sets
  `last_event_at = None`. Clearing `explicit_seen` re-arms Tier-3 + prompt
  markers; nulling `last_event_at` removes the stale-state baseline so the
  now-ungated stale-state timeout **cannot** silently reclaim Waiting→Active
  absent activity (without this, Waiting would be reclaimed ~`stale_state`
  after de-escalation — contradicting "persists"). Net: amber stays until a
  real transition moves it — the next shell prompt-marker / heuristic
  transition, or user interaction (next prompt → `UserPromptSubmit` →
  `working`). State is **not** force-changed by the fuse for Waiting.
- While explicit frames keep arriving (normal operation, incl. the 5-hook
  config refreshing on every PreToolUse/PostToolUse), `last_explicit_at` is
  refreshed and the fuse never fires — zero behavior change for healthy tools.
- `restart()` resets the whole tracker (fresh `AiStateTracker::new`) — no
  cross-restart preservation needed.

### Constraints / invariants
- Confined to `tracker.rs` (mechanism + field + tests) and the config layer
  (`config/schema.rs` + `config/mod.rs` `Ai`/`apply_ai` + `TrackerConfig`).
  `WindowApp::apply_config` already propagates `TrackerConfig` to sessions, so
  hot-reload and new-tab/restart inheritance come free via existing plumbing.
- Q1's existing behavior is unchanged when `explicit_stale_state == 0` or
  while signals keep arriving. The Stage-13 A1/A2 invariant is untouched
  (no color code).

### Tests (TDD)
- Fuse fires `Working`→`Active` after `explicit_stale_state` with no new frame
  (returns `true`).
- `Waiting` is **not** force-reset by the fuse; state stays `Waiting`.
- After the fuse, `explicit_seen` is `false` and Tier-3 resumes (e.g. a
  subsequent `OutputObserved` with `heuristic_active` promotes from
  `Active`/`Idle`; a `Prompt(CommandStart)` drives Working).
- A new `AiFrame` arriving *before* the fuse refreshes `last_explicit_at` →
  fuse does not fire prematurely.
- `explicit_stale_state == 0` disables the fuse (Q1 behavior preserved —
  `tick()` stays inert when explicit).
- All existing Q1 tests remain green (the new logic only adds a pre-check
  guarded by the new timer; default config in existing tests should keep them
  passing — verify; if a Q1 test would now fuse due to the default, it must
  set `explicit_stale_state: 0` or a large value, an intent-preserving
  precondition tweak, not a weakening).
- Config: `[ai] explicit_stale_state_s` parses; default 300; `0` round-trips;
  hot-reload propagates (mirror existing `[ai]` field tests).

---

## Workflow

Small spec → implementation plan (`writing-plans`) → senior pre-execution
Sonnet review of the plan vs actual source → `subagent-driven-development`
(fresh implementer + spec-compliance reviewer + code-quality reviewer per
task; reviewers constrained read-only / no git-fs mutation per the
`lesson_review_subagent_destructive` memory) → manual VNC smoke walk →
senior holistic review → merge to `main` (`--no-ff`, no separate tag —
folded into the v0.1/Stage-14 track). Controller runs `git status` after every
task (per `lesson_subagent_amend_drift`).

## Out of scope
- Tier-2 wrapper shims (`vibeflow-claude` etc.) — deferred for a few days of
  hook-config testing with claude first.
- Any further AI-state-fidelity work beyond the unified fuse — the residual is
  Claude Code hook coverage (now addressed in `~/.claude/settings.json`), not
  vibeflow code (see `lesson_osc1338_hook_coverage`).
