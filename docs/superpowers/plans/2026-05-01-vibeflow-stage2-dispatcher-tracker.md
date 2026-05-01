# vibeflow Stage 2 Implementation Plan: OscDispatcher + AiStateTracker

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the streaming OSC parser (`OscDispatcher`) and the per-tab state machine (`AiStateTracker`) — the second of six stages on the path to a working terminal. After this plan, the project has all the protocol-decoding plumbing it needs to drive tab visuals, but no PTY, window, or rendering yet.

**Architecture:** A new `vibeflow` library crate (no binary target yet) with a `session` module containing two pure Rust state machines:

- `OscDispatcher::feed(bytes) -> Vec<DispatchEvent>` consumes a byte stream incrementally, recognising `OSC 1338` (vibeflow's AI protocol) and `OSC 133` (shell prompt markers from iTerm/Terminal Integration), emitting `AiState`/`Prompt` events for those and `PassThrough` events for everything else (which Stage 3 forwards to alacritty's terminal grid).
- `AiStateTracker::on_input(input, now)` accepts `AiFrame`, `Prompt`, and `OutputObserved` inputs against an injected clock, runs a debounced state machine, and surfaces `tick(now)` for two timeout-driven transitions: heuristic silence (Tier 3 inference of `Waiting`) and stale state (forced reset to `Active` when a tool dies mid-task).

Both modules are pure functions of inputs, with zero I/O and no async. Stage 3 wires them together with a real PTY.

**Tech Stack:** Rust 2021 stable, depends on `vibeflow-protocol` (workspace-internal). No new external runtime deps. `proptest` for the dispatcher round-trip / never-panic property test (already a dev-dep at workspace level).

**Stage scope:** This plan covers Stage 2 only. After it ships, the natural next plan is Stage 3 (`PtySession` + reader thread + headless `App` glue). Stage 2 produces independently testable software: dispatcher + tracker can be exercised end-to-end via the integration test in Task 13, with no PTY or GUI required.

**Lessons carried from Stage 1:**
- Run `cargo fmt --all` after each implementation step before committing — rustfmt prefers wider line breaking than the human-readable plan style, and the CI workflow added in Stage 1 enforces fmt-clean. The verbatim code below is already fmt-clean for `rustfmt 1.x default`.
- Helpers whose first lib-level caller arrives in a *later* task in this same plan get `#[allow(dead_code)]` annotations on first introduction, with cleanup steps when the caller appears (avoiding clippy `-D warnings` failures on the intermediate commits).
- Doc comments containing `<placeholders>` must wrap them in code blocks — `cargo doc -D warnings` parses bare `<...>` as HTML tags.

---

## File Structure

| Path | Responsibility |
|---|---|
| `Cargo.toml` (workspace, modify) | Add `crates/vibeflow` to `members`. |
| `crates/vibeflow/Cargo.toml` (new) | Crate manifest. Depends on `vibeflow-protocol` via workspace path. |
| `crates/vibeflow/src/lib.rs` (new) | Top-level `vibeflow` library crate; declares the `session` module. Stage 3 will add a `[[bin]]` target alongside this lib. |
| `crates/vibeflow/src/session/mod.rs` (new) | The `session` module. Re-exports `osc::*` and `tracker::*` for the public API. |
| `crates/vibeflow/src/session/osc.rs` (new) | `OscDispatcher`, `DispatchEvent`, `PromptMarker`, OSC 133 body parser. |
| `crates/vibeflow/src/session/tracker.rs` (new) | `AiStateTracker`, `TabState`, `TrackerConfig`, `TrackerInput`, the `From<vibeflow_protocol::State>` conversion. |
| `crates/vibeflow/tests/integration.rs` (new, Task 13) | End-to-end: byte stream → dispatcher events → tracker → expected state sequence. |

---

## Task 0: Bootstrap `vibeflow` crate

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/vibeflow/Cargo.toml`
- Create: `crates/vibeflow/src/lib.rs`
- Create: `crates/vibeflow/src/session/mod.rs`

- [ ] **Step 1: Add the new crate to the workspace**

Edit `/home/bhengen/dev/vibeflow/Cargo.toml`. Find the line:

```toml
members = ["crates/vibeflow-protocol"]
```

Replace with:

```toml
members = ["crates/vibeflow", "crates/vibeflow-protocol"]
```

- [ ] **Step 2: Create the crate manifest**

Write `crates/vibeflow/Cargo.toml`:

```toml
[package]
name = "vibeflow"
description = "GPU-accelerated terminal emulator for Linux that knows when AI tools are waiting on the user (library crate, Stage 2 of v0.1)"
publish = false
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
authors.workspace = true
rust-version.workspace = true

[lints]
workspace = true

[lib]
path = "src/lib.rs"

[dependencies]
vibeflow-protocol = { path = "../vibeflow-protocol", version = "0.1" }

[dev-dependencies]
proptest = "1"
```

**Why `publish = false`:** `vibeflow` is the user-facing terminal binary, not a library to publish. Stage 3 will add a `[[bin]]` and Stage 4+ will eventually publish a binary release. For now the crate is a private workspace member only — `publish = false` means `cargo publish --workspace` skips it cleanly.

- [ ] **Step 3: Create `src/lib.rs`**

Write `crates/vibeflow/src/lib.rs`:

```rust
//! `vibeflow` — GPU-accelerated terminal emulator for Linux that signals AI-tool state.
//!
//! Stage 2 of v0.1 introduces only the streaming protocol dispatcher and the per-tab
//! state tracker; PTY, window, and rendering arrive in later stages. The current public
//! surface is the [`session`] module.
//!
//! See `docs/superpowers/specs/2026-05-01-vibeflow-design.md` for the full design.

pub mod session;
```

- [ ] **Step 4: Create `src/session/mod.rs`**

Write `crates/vibeflow/src/session/mod.rs` (you'll need to `mkdir -p crates/vibeflow/src/session`):

```rust
//! Per-tab session machinery: OSC dispatching, AI-state tracking.
//!
//! Stage 2 ships [`osc::OscDispatcher`] and [`tracker::AiStateTracker`]. Stage 3
//! adds a `pty` submodule that drives a real PTY child process and feeds its
//! output bytes through `OscDispatcher::feed`.

pub mod osc;
pub mod tracker;
```

- [ ] **Step 5: Verify the workspace builds**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
```

Expected: build will fail because `osc.rs` and `tracker.rs` don't exist yet. Sample output:

```
error[E0583]: file not found for module `osc`
error[E0583]: file not found for module `tracker`
```

This is expected; the next two steps create stub modules to fix it.

- [ ] **Step 6: Stub `osc.rs` and `tracker.rs`**

Write `crates/vibeflow/src/session/osc.rs`:

```rust
//! Streaming OSC dispatcher — recognises OSC 1338 (vibeflow's AI protocol) and
//! OSC 133 (shell-prompt integration), forwards everything else as pass-through
//! bytes for the terminal grid.
```

Write `crates/vibeflow/src/session/tracker.rs`:

```rust
//! Per-tab AI state tracker — debounced state machine with heuristic-silence
//! and stale-state timeouts.
```

- [ ] **Step 7: Re-verify the workspace builds clean**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: `Finished` with no errors and no clippy warnings.

- [ ] **Step 8: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add Cargo.toml crates/vibeflow/Cargo.toml crates/vibeflow/src/lib.rs crates/vibeflow/src/session/mod.rs crates/vibeflow/src/session/osc.rs crates/vibeflow/src/session/tracker.rs Cargo.lock
git commit -m "chore(vibeflow): bootstrap library crate with session module skeleton"
```

(`Cargo.lock` will have grown — workspace member addition forces a recheck. If `git status` shows it isn't dirty, drop it from the `git add`.)

---

## Task 1: `PromptMarker` + OSC 133 body parser (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`

The OSC 133 wire format (per the iTerm Terminal Integration spec) is:

```
ESC ] 133 ; <subtype> [ ; <param> ]* ( BEL | ST )
```

where `<subtype>` is `A` (prompt-start), `B` (prompt-end), `C` (command-start), or `D` (command-end). `D` may carry an optional numeric exit code as the next param. `A` and `B` may carry an `aid=<id>` param that we ignore.

This task adds the `PromptMarker` enum and the body parser. The parser takes the body *after* the `133;` prefix has been stripped.

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/vibeflow/src/session/osc.rs` with:

```rust
//! Streaming OSC dispatcher — recognises OSC 1338 (vibeflow's AI protocol) and
//! OSC 133 (shell-prompt integration), forwards everything else as pass-through
//! bytes for the terminal grid.

/// An OSC 133 "Terminal Integration" prompt marker.
///
/// Emitted when the dispatcher recognises one of the four standard subtypes.
/// Subtypes outside `A`/`B`/`C`/`D` are dropped silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMarker {
    /// `OSC 133;A` — start of prompt rendering.
    PromptStart,
    /// `OSC 133;B` — end of prompt; user can type now.
    PromptEnd,
    /// `OSC 133;C` — shell is about to run a command.
    CommandStart,
    /// `OSC 133;D[;<exit_code>]` — shell finished a command. `exit_code` is the
    /// command's status if the shell included it, otherwise `None`.
    CommandEnd { exit_code: Option<i32> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_133_prompt_start() {
        assert_eq!(parse_133_body("A"), Some(PromptMarker::PromptStart));
    }

    #[test]
    fn parse_133_prompt_end() {
        assert_eq!(parse_133_body("B"), Some(PromptMarker::PromptEnd));
    }

    #[test]
    fn parse_133_command_start() {
        assert_eq!(parse_133_body("C"), Some(PromptMarker::CommandStart));
    }

    #[test]
    fn parse_133_command_end_no_exit_code() {
        assert_eq!(
            parse_133_body("D"),
            Some(PromptMarker::CommandEnd { exit_code: None })
        );
    }

    #[test]
    fn parse_133_command_end_with_exit_code() {
        assert_eq!(
            parse_133_body("D;127"),
            Some(PromptMarker::CommandEnd {
                exit_code: Some(127)
            })
        );
    }

    #[test]
    fn parse_133_ignores_aid_on_prompt_start() {
        // iTerm's OSC 133;A;aid=<some-id> — we accept the subtype, ignore the aid
        assert_eq!(
            parse_133_body("A;aid=abc123"),
            Some(PromptMarker::PromptStart)
        );
    }

    #[test]
    fn parse_133_unknown_subtype_returns_none() {
        assert_eq!(parse_133_body("Z"), None);
        assert_eq!(parse_133_body(""), None);
    }

    #[test]
    fn parse_133_garbage_exit_code_falls_back_to_none() {
        // Non-numeric exit code: spec is silent, defensive behaviour is to
        // accept the CommandEnd marker but with no exit code.
        assert_eq!(
            parse_133_body("D;notanumber"),
            Some(PromptMarker::CommandEnd { exit_code: None })
        );
    }
}
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: compile error — `parse_133_body` not found.

- [ ] **Step 2: Implement `parse_133_body`**

Insert *above* the `#[cfg(test)] mod tests` block in `crates/vibeflow/src/session/osc.rs`:

```rust
/// Parse the body of an `OSC 133;…` sequence — the part *after* the `133;` prefix.
///
/// Returns `None` for unknown subtypes (caller drops the sequence). Garbage or
/// missing exit codes on `D` resolve to `CommandEnd { exit_code: None }`.
#[allow(dead_code)] // first caller arrives in Task 4 (OscDispatcher OSC 133 detection)
fn parse_133_body(body: &str) -> Option<PromptMarker> {
    let mut parts = body.split(';');
    let subtype = parts.next()?;
    match subtype {
        "A" => Some(PromptMarker::PromptStart),
        "B" => Some(PromptMarker::PromptEnd),
        "C" => Some(PromptMarker::CommandStart),
        "D" => {
            let exit_code = parts.next().and_then(|s| s.parse().ok());
            Some(PromptMarker::CommandEnd { exit_code })
        }
        _ => None,
    }
}
```

**Why `#[allow(dead_code)]` here:** the first lib-level caller of `parse_133_body` is the dispatcher's OSC 133 routing in Task 4. The tests in Task 1 do call it, but `clippy --all-targets -D warnings` checks the lib target alone before the test target. The lifecycle pattern matches Stage 1's percent-encoding helpers (added in Task 3, attribute removed in Tasks 4–5).

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 8 passed; 0 failed`.

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: both silent (no output, exit 0).

If `cargo fmt --check` fails, run `cargo fmt --all` to apply the formatting and re-verify.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/osc.rs
git commit -m "feat(session): add PromptMarker and OSC 133 body parser"
```

---

## Task 2: `OscDispatcher` skeleton + pass-through-only feed (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`

This task adds the public types (`DispatchEvent`, `OscDispatcher`) and implements the simplest case: when the input contains no OSC sequences, `feed` returns a single `PassThrough` event with all the input bytes verbatim.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `crates/vibeflow/src/session/osc.rs`:

```rust
    #[test]
    fn dispatcher_passes_plain_text_through_unchanged() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"hello, world");
        assert_eq!(events, vec![DispatchEvent::PassThrough(b"hello, world".to_vec())]);
    }

    #[test]
    fn dispatcher_passes_empty_input_through_with_no_events() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"");
        assert_eq!(events, vec![]);
    }

    #[test]
    fn dispatcher_passes_through_lone_esc_at_end_of_buffer() {
        // ESC at the end of a chunk is held internally, not emitted yet — but
        // at this stage of the plan we don't yet have the "emit ESC if next
        // byte isn't `]`" path. The simplest behaviour: an ESC that doesn't
        // form an OSC introducer is held in internal state until the next
        // feed call resolves it. Test the "no OSC came after" path: an ESC
        // followed by a non-`]` byte in a SINGLE feed is just passthrough.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"a\x1bb"); // ESC followed by 'b' (not ']') — passthrough as-is
        assert_eq!(events, vec![DispatchEvent::PassThrough(b"a\x1bb".to_vec())]);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: compile errors — `OscDispatcher`, `DispatchEvent` not found.

- [ ] **Step 2: Add public types and the skeleton state machine**

Insert *above* the `#[cfg(test)] mod tests` block:

```rust
use vibeflow_protocol::Frame;

/// Maximum total length of a single OSC sequence (including `ESC ]` and the
/// terminator). Sequences exceeding this are dropped on the floor.
const MAX_OSC_LEN: usize = 4096;

/// One event emitted by [`OscDispatcher::feed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchEvent {
    /// A complete OSC 1338 frame was parsed.
    AiState(Frame),
    /// An OSC 133 prompt marker was identified.
    Prompt(PromptMarker),
    /// Bytes that should be forwarded to the terminal grid (alacritty_terminal in
    /// future stages). Includes any unknown OSC sequences (their original bytes,
    /// terminator and all) plus all non-OSC bytes.
    PassThrough(Vec<u8>),
}

/// Internal parser state. Tracks whether we're scanning plain bytes, have just
/// seen an `ESC`, are inside an OSC body buffering toward the terminator, or
/// have seen an `ESC` *inside* an OSC body (potential start of `ESC \` ST).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    Plain,
    SeenEsc,
    InOsc,
    InOscEsc, // ESC inside OSC; if next byte is `\`, terminate as ST
}

/// Streaming OSC dispatcher.
///
/// Feed bytes incrementally with [`OscDispatcher::feed`]; each call returns a
/// `Vec<DispatchEvent>` ordered by where each event falls in the input. Internal
/// state is preserved across calls so partial sequences split across reads are
/// handled correctly.
#[derive(Debug)]
pub struct OscDispatcher {
    state: ParseState,
    /// Bytes seen so far in the current OSC body (after `ESC ]`, before terminator).
    osc_body: Vec<u8>,
    /// Pending pass-through bytes accumulated since the last emitted event.
    /// Flushed at the end of each `feed` call (or when an OSC starts).
    pass_buf: Vec<u8>,
    /// True once the current OSC body has overflowed `MAX_OSC_LEN`. We keep
    /// scanning for the terminator but discard the body and emit nothing.
    osc_overflowed: bool,
}

impl OscDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: ParseState::Plain,
            osc_body: Vec::with_capacity(64),
            pass_buf: Vec::with_capacity(256),
            osc_overflowed: false,
        }
    }

    /// Feed a chunk of bytes into the dispatcher; returns events in input order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<DispatchEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            self.step(b, &mut events);
        }
        // Flush any pending pass-through at the end of the chunk.
        if !self.pass_buf.is_empty() {
            events.push(DispatchEvent::PassThrough(std::mem::take(&mut self.pass_buf)));
        }
        events
    }

    /// Process a single byte. State transitions only — no allocation in the
    /// hot path beyond the single `pass_buf` push per non-OSC byte.
    fn step(&mut self, b: u8, _events: &mut Vec<DispatchEvent>) {
        match self.state {
            ParseState::Plain => {
                if b == 0x1B {
                    self.state = ParseState::SeenEsc;
                } else {
                    self.pass_buf.push(b);
                }
            }
            ParseState::SeenEsc => {
                // We deferred the ESC byte. At Stage 2 of this plan, OSC entry
                // (next byte is `]`) lands in Task 3; for now, any byte after
                // ESC just resolves back to plain pass-through with the ESC
                // restored.
                self.pass_buf.push(0x1B);
                self.pass_buf.push(b);
                self.state = ParseState::Plain;
            }
            ParseState::InOsc | ParseState::InOscEsc => {
                // OSC parsing arrives in Task 3; for Stage 2, this branch is
                // unreachable because we never enter InOsc.
                unreachable!("OSC parsing not implemented until Task 3");
            }
        }
    }
}

impl Default for OscDispatcher {
    fn default() -> Self {
        Self::new()
    }
}
```

**Why a `step` helper that takes `&mut Vec<DispatchEvent>` even though Task 2 doesn't push events:** future tasks (3, 4, 5) will push events from the OSC-recognition branches. Pre-establishing the signature avoids an awkward refactor later. The `_events` underscore prefix silences the unused-parameter clippy warning for the Stage 2 commit.

**Why `unreachable!` rather than `panic!` or silently doing nothing:** `unreachable!` documents the invariant that Task 2 doesn't construct `InOsc`. If Task 3 forgets to set the state correctly, this panics in tests — a fast, loud failure mode. The OscDispatcher's spec-level guarantee ("never panic") only kicks in once the full state machine is implemented (Task 5+). For Task 2's intermediate state, a debug-time assert is appropriate. Production callers don't reach this branch unless Task 3's caller paths are wrong.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 11 passed; 0 failed`. (8 from Task 1 + 3 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent, exit 0. If fmt fails, run `cargo fmt --all` then re-verify.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/osc.rs
git commit -m "feat(session): add OscDispatcher skeleton and DispatchEvent type"
```

---

## Task 3: `OscDispatcher` recognises OSC 1338 (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`

Adds the OSC entry path (ESC `]` after a deferred ESC), buffers the body until `BEL` or `ESC \`, then dispatches the recognised OSC 1338 sequences via `vibeflow_protocol::parse`.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    use vibeflow_protocol::State;

    #[test]
    fn dispatcher_recognises_osc_1338_bel_terminated() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=working\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(Frame::new(State::Working))]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_1338_with_tool_and_project() {
        let mut d = OscDispatcher::new();
        let events =
            d.feed(b"\x1b]1338;state=waiting;tool=claude;project=vibeflow\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(
                Frame::new(State::Waiting)
                    .with_tool("claude")
                    .with_project("vibeflow")
            )]
        );
    }

    #[test]
    fn dispatcher_recognises_osc_1338_st_terminated() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=active\x1b\\");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(Frame::new(State::Active))]
        );
    }

    #[test]
    fn dispatcher_emits_passthrough_around_osc_1338() {
        let mut d = OscDispatcher::new();
        let events =
            d.feed(b"hello\x1b]1338;state=working\x07world");
        assert_eq!(
            events,
            vec![
                DispatchEvent::PassThrough(b"hello".to_vec()),
                DispatchEvent::AiState(Frame::new(State::Working)),
                DispatchEvent::PassThrough(b"world".to_vec()),
            ]
        );
    }

    #[test]
    fn dispatcher_handles_double_esc_followed_by_osc() {
        // ESC ESC ] is "first ESC was a false start, second ESC is the real
        // introducer". The first ESC should land in passthrough; the OSC
        // should still be recognised.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b\x1b]1338;state=working\x07");
        assert_eq!(
            events,
            vec![
                DispatchEvent::PassThrough(b"\x1b".to_vec()),
                DispatchEvent::AiState(Frame::new(State::Working)),
            ]
        );
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 5 new tests fail at runtime (panic on `unreachable!` from Task 2's `InOsc` branch when entering OSC body). The first test will hit the panic immediately on the second byte (`]`).

- [ ] **Step 2: Replace the `step` body to handle OSC entry, buffering, and termination**

Replace the entire `fn step` method on `OscDispatcher` with:

```rust
    /// Process a single byte. State transitions only.
    fn step(&mut self, b: u8, events: &mut Vec<DispatchEvent>) {
        match self.state {
            ParseState::Plain => {
                if b == 0x1B {
                    self.state = ParseState::SeenEsc;
                } else {
                    self.pass_buf.push(b);
                }
            }
            ParseState::SeenEsc => {
                if b == b']' {
                    // OSC introducer — flush pending passthrough first so events
                    // arrive in input order, then enter OSC body parsing.
                    self.flush_pass(events);
                    self.state = ParseState::InOsc;
                    self.osc_body.clear();
                    self.osc_overflowed = false;
                } else if b == 0x1B {
                    // Two ESCs in a row. The first ESC was a false start (this
                    // byte is ESC, not `]`). Emit the first ESC as plain and
                    // treat this second ESC as a fresh OSC-introducer candidate.
                    self.pass_buf.push(0x1B);
                    // state stays SeenEsc with this ESC pending
                } else {
                    // Not an OSC — restore ESC + this byte as plain bytes.
                    self.pass_buf.push(0x1B);
                    self.pass_buf.push(b);
                    self.state = ParseState::Plain;
                }
            }
            ParseState::InOsc => {
                if b == 0x07 {
                    // BEL terminator
                    self.finish_osc(events);
                } else if b == 0x1B {
                    // Could be the start of an `ESC \` ST terminator
                    self.state = ParseState::InOscEsc;
                } else {
                    self.push_osc_byte(b);
                }
            }
            ParseState::InOscEsc => {
                if b == b'\\' {
                    // ESC \ — ST terminator
                    self.finish_osc(events);
                } else {
                    // ESC inside an OSC body that didn't form ST — treat the
                    // ESC as starting a new OSC introducer attempt; drop the
                    // current OSC (we have no way to recover its terminator).
                    // This is the "malformed OSC" path. The current byte still
                    // needs to be processed: re-feed it from a fresh state.
                    self.osc_body.clear();
                    self.osc_overflowed = false;
                    self.state = ParseState::SeenEsc;
                    self.step(b, events);
                }
            }
        }
    }
```

Then add the four helpers below `step` (still inside `impl OscDispatcher`):

```rust
    fn flush_pass(&mut self, events: &mut Vec<DispatchEvent>) {
        if !self.pass_buf.is_empty() {
            events.push(DispatchEvent::PassThrough(std::mem::take(&mut self.pass_buf)));
        }
    }

    fn push_osc_byte(&mut self, b: u8) {
        if self.osc_overflowed {
            return;
        }
        // +2 accounts for the ESC ] header that's not in osc_body but counts
        // toward MAX_OSC_LEN; +1 for the terminator we'll see soon.
        if self.osc_body.len() + 3 >= MAX_OSC_LEN {
            self.osc_overflowed = true;
            return;
        }
        self.osc_body.push(b);
    }

    fn finish_osc(&mut self, events: &mut Vec<DispatchEvent>) {
        let body = std::mem::take(&mut self.osc_body);
        let overflowed = std::mem::replace(&mut self.osc_overflowed, false);
        self.state = ParseState::Plain;

        if overflowed {
            // Spec: "over-long sequences are dropped on the floor". No event.
            return;
        }

        if let Some(event) = handle_osc(&body) {
            events.push(event);
        }
        // If `handle_osc` returned None, that's a malformed-or-unknown OSC.
        // For now (Task 3) we drop. Task 5 reintroduces unknown-OSC
        // pass-through.
    }
```

Then add the `handle_osc` helper as a free function below the `impl OscDispatcher` block:

```rust
/// Route a complete OSC body (the bytes between `ESC ]` and the terminator) to
/// the appropriate handler. Returns `None` for unknown OSCs and for OSC 1338
/// sequences that fail to parse.
fn handle_osc(body: &[u8]) -> Option<DispatchEvent> {
    let body_str = std::str::from_utf8(body).ok()?;
    let (id, _params) = body_str.split_once(';').unwrap_or((body_str, ""));
    match id {
        "1338" => {
            // Reconstruct the full sequence and hand it to the protocol crate.
            let mut full = Vec::with_capacity(body.len() + 3);
            full.push(0x1B);
            full.push(b']');
            full.extend_from_slice(body);
            full.push(0x07);
            vibeflow_protocol::parse(&full)
                .ok()
                .map(DispatchEvent::AiState)
        }
        _ => None,
    }
}
```

**Why reconstruct the full sequence for `vibeflow_protocol::parse`:** the protocol crate's `parse` is the public API and the contract tested by 27 unit tests + a proptest. Reusing it (rather than parsing the body inline) means the dispatcher and the protocol library can never disagree about what counts as a valid OSC 1338 frame. The cost is one allocation per OSC 1338 sequence — fine for the ~once-per-state-change frequency.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 16 passed; 0 failed`. (11 from Task 2 + 5 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent. If fmt fails, run `cargo fmt --all` and re-verify.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/osc.rs
git commit -m "feat(session): OscDispatcher recognises OSC 1338 frames"
```

---

## Task 4: `OscDispatcher` recognises OSC 133 (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`

Extends `handle_osc` to route OSC 133 sequences through `parse_133_body`. Removes the `#[allow(dead_code)]` attribute on `parse_133_body` because it's now reachable from lib code.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn dispatcher_recognises_osc_133_prompt_start() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;A\x07");
        assert_eq!(events, vec![DispatchEvent::Prompt(PromptMarker::PromptStart)]);
    }

    #[test]
    fn dispatcher_recognises_osc_133_command_start() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;C\x07");
        assert_eq!(events, vec![DispatchEvent::Prompt(PromptMarker::CommandStart)]);
    }

    #[test]
    fn dispatcher_recognises_osc_133_command_end_with_exit_code() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;D;127\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::Prompt(PromptMarker::CommandEnd {
                exit_code: Some(127)
            })]
        );
    }

    #[test]
    fn dispatcher_drops_osc_133_with_unknown_subtype() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]133;Z\x07");
        // No event; OSC 133 with unknown subtype is recognised-and-dropped.
        // Task 5 will distinguish this from completely unknown OSCs (which
        // become PassThrough).
        assert_eq!(events, vec![]);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 4 new tests fail (`handle_osc` returns None for `133`, so no event is emitted).

- [ ] **Step 2: Add `133` arm to `handle_osc` and remove the dead-code allow**

Find the `handle_osc` function and replace its body with:

```rust
fn handle_osc(body: &[u8]) -> Option<DispatchEvent> {
    let body_str = std::str::from_utf8(body).ok()?;
    let (id, params) = body_str.split_once(';').unwrap_or((body_str, ""));
    match id {
        "1338" => {
            let mut full = Vec::with_capacity(body.len() + 3);
            full.push(0x1B);
            full.push(b']');
            full.extend_from_slice(body);
            full.push(0x07);
            vibeflow_protocol::parse(&full)
                .ok()
                .map(DispatchEvent::AiState)
        }
        "133" => parse_133_body(params).map(DispatchEvent::Prompt),
        _ => None,
    }
}
```

Then find this line above `fn parse_133_body`:

```rust
#[allow(dead_code)] // first caller arrives in Task 4 (OscDispatcher OSC 133 detection)
```

Delete it. The function is now called from `handle_osc` (lib code), so the allow is no longer needed.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 20 passed; 0 failed`. (16 from Task 3 + 4 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/osc.rs
git commit -m "feat(session): OscDispatcher recognises OSC 133 prompt markers"
```

---

## Task 5: `OscDispatcher` edge cases — unknown OSC pass-through, oversize, multi-call (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`

Three edge cases:
1. **Unknown OSCs** (e.g. `OSC 0;<title>` for window-title — alacritty handles this) must be passed through *with their original bytes intact*, not silently dropped. Without this, the terminal grid loses functionality the moment the dispatcher sees a non-1338-non-133 OSC.
2. **Oversize sequences** (>4 KiB) are dropped, matching the spec's `MAX_OSC_LEN`.
3. **Multi-call buffering**: a sequence split across two `feed` calls is parsed correctly when the second call arrives.

The dispatcher already implements (2) via `osc_overflowed` and (3) via the persistent state. This task adds (1) by changing `handle_osc`'s contract: distinguishing "recognised, no event" (drop) from "unknown OSC" (pass-through).

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn dispatcher_passes_through_unknown_osc_intact() {
        // OSC 0 is the iTerm/xterm window-title sequence. We don't recognise
        // it, so the original bytes (ESC ] 0;<title> BEL) must reach the
        // terminal grid unchanged.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]0;hello world\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(b"\x1b]0;hello world\x07".to_vec())]
        );
    }

    #[test]
    fn dispatcher_passes_unknown_osc_with_st_terminator_intact() {
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]7;file://example\x1b\\");
        assert_eq!(
            events,
            vec![DispatchEvent::PassThrough(b"\x1b]7;file://example\x1b\\".to_vec())]
        );
    }

    #[test]
    fn dispatcher_drops_oversize_osc() {
        let mut d = OscDispatcher::new();
        // Build a single OSC 1338 sequence whose body is well over MAX_OSC_LEN.
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b]1338;state=waiting;tool=");
        input.extend(std::iter::repeat(b'x').take(5000));
        input.push(0x07);
        let events = d.feed(&input);
        // Oversize → silently dropped; no events.
        assert_eq!(events, vec![]);
    }

    #[test]
    fn dispatcher_handles_osc_split_across_two_feeds() {
        let mut d = OscDispatcher::new();
        let first = d.feed(b"hello\x1b]1338;state=");
        // The "hello" passthrough flushes at end of feed; the OSC body has
        // started and stays in internal state.
        assert_eq!(
            first,
            vec![DispatchEvent::PassThrough(b"hello".to_vec())]
        );
        let second = d.feed(b"working\x07world");
        assert_eq!(
            second,
            vec![
                DispatchEvent::AiState(Frame::new(State::Working)),
                DispatchEvent::PassThrough(b"world".to_vec()),
            ]
        );
    }

    #[test]
    fn dispatcher_recovers_from_malformed_osc() {
        // ESC `inside` an OSC body that doesn't form ST → drop the current
        // OSC and start a fresh OSC parse from the new ESC. The new OSC
        // (state=waiting) parses cleanly.
        let mut d = OscDispatcher::new();
        let events = d.feed(b"\x1b]1338;state=garbage\x1b]1338;state=waiting\x07");
        assert_eq!(
            events,
            vec![DispatchEvent::AiState(Frame::new(State::Waiting))]
        );
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 2 new tests fail (`dispatcher_passes_through_unknown_osc_intact` and the ST variant — both will return empty events because the current `handle_osc` drops unknown OSCs). The other 3 should already pass:
- `dispatcher_drops_oversize_osc` (overflow path is in place from Task 3)
- `dispatcher_handles_osc_split_across_two_feeds` (state persistence is in place from Task 2/3)
- `dispatcher_recovers_from_malformed_osc` (ESC-inside-OSC recovery from Task 3 step 2)

If any of these three fail, that's a real bug from earlier tasks — STOP and report.

- [ ] **Step 2: Change `handle_osc` to a tri-state outcome and pipe pass-through bytes through**

Replace the entire `fn handle_osc` block with:

```rust
/// What `handle_osc` decided to do with a complete OSC body.
enum OscOutcome {
    /// We recognised the OSC and produced this event.
    Event(DispatchEvent),
    /// We recognised the OSC ID but the body was malformed for it (e.g. an
    /// OSC 1338 with an unknown state value). Drop silently — log debug in
    /// future stages.
    Drop,
    /// We don't own this OSC ID. Caller should emit a PassThrough with the
    /// original bytes (ESC ] body terminator) intact.
    Forward,
}

fn handle_osc(body: &[u8]) -> OscOutcome {
    let Some(body_str) = std::str::from_utf8(body).ok() else {
        // Non-UTF-8 body. We don't own this OSC; let the terminal try.
        return OscOutcome::Forward;
    };
    let (id, params) = body_str.split_once(';').unwrap_or((body_str, ""));
    match id {
        "1338" => {
            let mut full = Vec::with_capacity(body.len() + 3);
            full.push(0x1B);
            full.push(b']');
            full.extend_from_slice(body);
            full.push(0x07);
            match vibeflow_protocol::parse(&full) {
                Ok(frame) => OscOutcome::Event(DispatchEvent::AiState(frame)),
                Err(_) => OscOutcome::Drop,
            }
        }
        "133" => match parse_133_body(params) {
            Some(marker) => OscOutcome::Event(DispatchEvent::Prompt(marker)),
            None => OscOutcome::Drop,
        },
        _ => OscOutcome::Forward,
    }
}
```

Then replace `fn finish_osc` to handle the new tri-state outcome and reconstruct the original bytes when forwarding. Find and replace this method on `impl OscDispatcher`:

```rust
    fn finish_osc(&mut self, events: &mut Vec<DispatchEvent>) {
        let body = std::mem::take(&mut self.osc_body);
        let overflowed = std::mem::replace(&mut self.osc_overflowed, false);
        // Track which terminator we saw — needed to reconstruct an unknown OSC.
        // self.state at this point is either InOsc (BEL termination) or
        // InOscEsc (ESC \ termination).
        let used_st = self.state == ParseState::InOscEsc;
        self.state = ParseState::Plain;

        if overflowed {
            return;
        }

        match handle_osc(&body) {
            OscOutcome::Event(ev) => events.push(ev),
            OscOutcome::Drop => {}
            OscOutcome::Forward => {
                // Reconstruct the original sequence and emit as PassThrough.
                let mut full = Vec::with_capacity(body.len() + 4);
                full.push(0x1B);
                full.push(b']');
                full.extend_from_slice(&body);
                if used_st {
                    full.push(0x1B);
                    full.push(b'\\');
                } else {
                    full.push(0x07);
                }
                events.push(DispatchEvent::PassThrough(full));
            }
        }
    }
```

**Why reconstruct on Forward:** the dispatcher consumed the original bytes one-by-one as it scanned for the terminator. To pass through to alacritty, we have to rebuild them. We know we always saw `ESC ]` (introducer) and either `BEL` or `ESC \` (terminator) — that's enough to round-trip. Bytes inside the body went through `push_osc_byte` and are in `body`.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 25 passed; 0 failed`. (20 from Task 4 + 5 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/osc.rs
git commit -m "feat(session): unknown-OSC pass-through, oversize drop, multi-call buffering"
```

---

## Task 6: `OscDispatcher` never-panic property test

**Files:**
- Modify: `crates/vibeflow/src/session/osc.rs`

Add a proptest that feeds arbitrary byte sequences through the dispatcher and asserts it never panics or OOMs. Mirrors the cargo-fuzz contract from Stage 1 but inside the unit-test runner — so it runs on every CI build, not just the fuzz job. (Stage 4+ may add a separate cargo-fuzz target for the dispatcher in addition.)

- [ ] **Step 1: Add the property test**

Append to the `mod tests` block:

```rust
    use proptest::prelude::*;

    proptest! {
        /// Feeding arbitrary bytes through the dispatcher in arbitrary chunk
        /// sizes must never panic and must never produce more bytes of
        /// PassThrough output than were fed in (the dispatcher has no source
        /// of expansion: every byte either feeds an OSC body, becomes part of
        /// a passthrough, or is dropped via overflow).
        #[test]
        fn dispatcher_never_panics_on_arbitrary_input(
            chunks in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..200),
                0..10,
            ),
        ) {
            let mut d = OscDispatcher::new();
            let mut total_input: usize = 0;
            let mut total_passthrough: usize = 0;
            for chunk in &chunks {
                total_input += chunk.len();
                for ev in d.feed(chunk) {
                    if let DispatchEvent::PassThrough(bytes) = ev {
                        total_passthrough += bytes.len();
                    }
                }
            }
            // PassThrough output cannot exceed total bytes fed in. (Equality
            // when no OSC was recognised; less when OSC bodies were consumed.)
            prop_assert!(total_passthrough <= total_input);
        }
    }
```

- [ ] **Step 2: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 26 passed; 0 failed`. (25 from Task 5 + 1 new — the proptest counts as a single test name running 256 cases by default.)

If proptest finds a panicking input, it shrinks to a minimal counter-example and prints it. That's a real bug in the dispatcher — STOP and report.

- [ ] **Step 3: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/osc.rs
git commit -m "test(session): proptest — OscDispatcher never panics on arbitrary input"
```

---

## Task 7: `TabState`, `TrackerConfig`, `From<vibeflow_protocol::State>` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs`

The state space the tracker manages is a strict superset of the protocol's: it adds `Idle` (shell at prompt, nothing running) which the protocol can't carry directly. This task adds those types and the `From` conversion.

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/vibeflow/src/session/tracker.rs` with:

```rust
//! Per-tab AI state tracker — debounced state machine with heuristic-silence
//! and stale-state timeouts.

use std::time::Duration;

use vibeflow_protocol::State;

/// Visual state of a single tab/session.
///
/// A strict superset of [`vibeflow_protocol::State`]: adds [`TabState::Idle`]
/// for "shell at prompt, no command running", which the OSC 1338 protocol
/// cannot carry (only AI tools emit OSC 1338, and an idle shell isn't one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TabState {
    /// Default — nothing notable is happening.
    #[default]
    Active,
    /// A tool/shell is running a command.
    Working,
    /// A tool is waiting for user input. The headline state.
    Waiting,
    /// A tool just finished a task; usually transient.
    Done,
    /// Shell at prompt, nothing running.
    Idle,
}

impl From<State> for TabState {
    fn from(s: State) -> Self {
        match s {
            State::Active => TabState::Active,
            State::Working => TabState::Working,
            State::Waiting => TabState::Waiting,
            State::Done => TabState::Done,
        }
    }
}

/// Tunable thresholds for the tracker. Mirrors the `[ai]` section of vibeflow's
/// TOML config (added in a later stage); defaults match the design spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackerConfig {
    /// Ignore state transitions closer together than this. Spec default 100 ms.
    pub debounce: Duration,
    /// Tier 3 fallback: infer `Waiting` after this much output silence on a
    /// session whose foreground process is in the configured AI-tool list.
    /// Spec default 4000 ms.
    pub heuristic_silence: Duration,
    /// Reset to `Active` if a tool emits a state but never updates again — guards
    /// against stuck indicators when a tool dies mid-task. Spec default 30 s.
    pub stale_state: Duration,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(100),
            heuristic_silence: Duration::from_millis(4000),
            stale_state: Duration::from_secs(30),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_state_converts_to_tab_state() {
        assert_eq!(TabState::from(State::Active), TabState::Active);
        assert_eq!(TabState::from(State::Working), TabState::Working);
        assert_eq!(TabState::from(State::Waiting), TabState::Waiting);
        assert_eq!(TabState::from(State::Done), TabState::Done);
    }

    #[test]
    fn tracker_config_defaults_match_spec() {
        let c = TrackerConfig::default();
        assert_eq!(c.debounce, Duration::from_millis(100));
        assert_eq!(c.heuristic_silence, Duration::from_millis(4000));
        assert_eq!(c.stale_state, Duration::from_secs(30));
    }

    #[test]
    fn tab_state_default_is_active() {
        assert_eq!(TabState::default(), TabState::Active);
    }
}
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 3 new tests pass (no implementation needed; this task only introduces value-types). Total `test result: ok. 29 passed`.

- [ ] **Step 2: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(session): add TabState, TrackerConfig, and State conversion"
```

---

## Task 8: `AiStateTracker` — `AiFrame` input + `state()` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs`

The tracker proper. Constructor, the `TrackerInput` enum, the `on_input(input, now)` method for `AiFrame` events, and `state()` accessor. No debounce or timeouts yet — those are Tasks 10–12.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    use std::time::Instant;
    use vibeflow_protocol::Frame;

    #[test]
    fn tracker_starts_in_active_state() {
        let t = AiStateTracker::new(TrackerConfig::default());
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_transitions_to_working_on_ai_frame() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_transitions_to_waiting_on_ai_frame() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // Use a long delay past the debounce window so this test is unaffected
        // when debounce is added in Task 10.
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_returns_false_when_state_unchanged() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // Tracker starts in Active; sending an Active frame must not register
        // as a change.
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Active)),
            now,
        );
        assert!(!changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_handles_output_observed_without_changing_state() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            Instant::now(),
        );
        let changed = t.on_input(TrackerInput::OutputObserved, Instant::now());
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: compile errors — `AiStateTracker` and `TrackerInput` not found.

- [ ] **Step 2: Implement `AiStateTracker` + `TrackerInput`**

Below the `TrackerConfig` block (and above `#[cfg(test)] mod tests`), insert:

```rust
use std::time::Instant;

use vibeflow_protocol::Frame;

use crate::session::osc::PromptMarker;

/// Inputs the tracker reacts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackerInput {
    /// An OSC 1338 frame from a tool — directly drives state.
    AiFrame(Frame),
    /// An OSC 133 prompt marker from the shell — used to derive Idle/Working
    /// when no AI tool is active.
    Prompt(PromptMarker),
    /// "Output bytes observed at `now`" — used by heuristic silence detection.
    /// Doesn't directly change state; just resets the silence timer.
    OutputObserved,
}

/// Per-tab state machine: tracks the current [`TabState`], applies debounce,
/// and surfaces stale-state and heuristic-silence timeouts via [`tick`].
///
/// Time is injected as an explicit `now: Instant` argument on every method
/// rather than read from the system clock. This keeps the tracker a pure
/// function of its inputs (testable without sleeping) and matches the
/// single-thread main-loop call site that already has `Instant::now()` in hand.
///
/// [`tick`]: AiStateTracker::tick
#[derive(Debug)]
pub struct AiStateTracker {
    state: TabState,
    config: TrackerConfig,
    /// `Instant` of the last input that affected state. `None` until the first
    /// state transition.
    last_event_at: Option<Instant>,
    /// `Instant` of the last `OutputObserved` input. `None` until first observed.
    last_output_at: Option<Instant>,
    /// Set externally by the App (Stage 3+) when the foreground process matches
    /// the configured AI-tool list. Drives Tier 3 heuristic silence inference.
    heuristic_active: bool,
}

impl AiStateTracker {
    #[must_use]
    pub fn new(config: TrackerConfig) -> Self {
        Self {
            state: TabState::default(),
            config,
            last_event_at: None,
            last_output_at: None,
            heuristic_active: false,
        }
    }

    /// Current visual state.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.state
    }

    /// Apply an input at `now`. Returns `true` if the state changed.
    pub fn on_input(&mut self, input: TrackerInput, now: Instant) -> bool {
        match input {
            TrackerInput::AiFrame(frame) => self.transition_to(frame.state.into(), now),
            TrackerInput::Prompt(marker) => {
                let _ = marker;
                // Prompt-driven transitions land in Task 9.
                false
            }
            TrackerInput::OutputObserved => {
                self.last_output_at = Some(now);
                false
            }
        }
    }

    /// Stale-state and heuristic-silence checks at `now`. Returns `true` if a
    /// timeout caused a state change. (Stub for Task 8; real logic in Tasks 11–12.)
    #[allow(dead_code)] // first lib-level caller arrives in Task 11
    pub fn tick(&mut self, now: Instant) -> bool {
        let _ = now;
        false
    }

    /// Toggle the Tier 3 heuristic — set true when the foreground process is
    /// in the configured AI-tool list, false otherwise.
    #[allow(dead_code)] // first lib-level caller is in the App in Stage 3
    pub fn set_heuristic_active(&mut self, active: bool) {
        self.heuristic_active = active;
    }

    /// Internal: change state if the new value differs and (Task 10+) debounce
    /// allows. Returns true if the state actually changed.
    fn transition_to(&mut self, new_state: TabState, now: Instant) -> bool {
        if self.state == new_state {
            return false;
        }
        self.state = new_state;
        self.last_event_at = Some(now);
        true
    }
}
```

**Why expose `tick` and `set_heuristic_active` with `#[allow(dead_code)]` already:** these are part of the public API the App will use in Stage 3. Defining them now (even as stubs) lets Tasks 11/12 fill in the body without churning the signature. The allow attributes are removed in those tasks.

**Why `TrackerInput::Prompt` returns false in this task instead of just panicking like the OSC dispatcher's stub:** prompt handling (Task 9) needs to coexist with `AiFrame` handling, and the tracker's API contract says `on_input` is total — never panics. Returning `false` (no state change) for an input we'll handle properly in the very next task is safer than a runtime crash. The `let _ = marker;` silences the unused-variable clippy warning.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 34 passed; 0 failed`. (29 from Task 7 + 5 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(session): AiStateTracker skeleton with AiFrame transitions"
```

---

## Task 9: `AiStateTracker` — `Prompt` input handling (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs`

OSC 133 prompt markers drive shell-state inference: PromptStart and CommandEnd → `Idle`, CommandStart → `Working`. (PromptEnd is informational; same effect as PromptStart for our purposes — the user can type, nothing's running, state is Idle.)

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn tracker_transitions_to_idle_on_prompt_start() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        // First, transition out of the default Active so we can observe the
        // change to Idle.
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptStart),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Idle);
    }

    #[test]
    fn tracker_transitions_to_idle_on_prompt_end() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptEnd),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Idle);
    }

    #[test]
    fn tracker_transitions_to_working_on_command_start() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        // Start by going to Idle so the Working transition is observable.
        t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptStart),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandStart),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_transitions_to_idle_on_command_end() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandStart),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandEnd { exit_code: Some(0) }),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Idle);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 4 new tests fail — Prompt handling currently returns `false` from Task 8.

- [ ] **Step 2: Implement Prompt-driven transitions**

In `fn on_input`, replace the `TrackerInput::Prompt(marker) => …` arm with:

```rust
            TrackerInput::Prompt(marker) => {
                let target = match marker {
                    PromptMarker::PromptStart
                    | PromptMarker::PromptEnd
                    | PromptMarker::CommandEnd { .. } => TabState::Idle,
                    PromptMarker::CommandStart => TabState::Working,
                };
                self.transition_to(target, now)
            }
```

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 38 passed; 0 failed`. (34 from Task 8 + 4 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(session): tracker derives Idle/Working from OSC 133 prompts"
```

---

## Task 10: `AiStateTracker` — debounce (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs`

Spec: "Debounces flapping (<100 ms transitions ignored)". Transitions closer together than `config.debounce` are suppressed — the second transition is dropped, the state stays at whatever the first transition put it in.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn tracker_suppresses_flapping_within_debounce_window() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // First transition at `now` — accepted.
        let c1 = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        assert!(c1);
        assert_eq!(t.state(), TabState::Working);
        // Second transition 50 ms later — within the 100 ms debounce window;
        // suppressed. State must remain Working.
        let c2 = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_millis(50),
        );
        assert!(!c2);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_accepts_transitions_past_debounce_window() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_millis(150),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_debounce_is_configurable() {
        let mut t = AiStateTracker::new(TrackerConfig {
            debounce: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        // 200 ms — within the custom 500 ms debounce, so suppressed.
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_millis(200),
        );
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 3 new tests fail — debounce isn't enforced yet.

- [ ] **Step 2: Add debounce check to `transition_to`**

Replace `fn transition_to` with:

```rust
    fn transition_to(&mut self, new_state: TabState, now: Instant) -> bool {
        if self.state == new_state {
            return false;
        }
        // Debounce: suppress transitions closer together than `config.debounce`.
        // The first transition (last_event_at == None) is always accepted.
        if let Some(last) = self.last_event_at {
            if now.saturating_duration_since(last) < self.config.debounce {
                return false;
            }
        }
        self.state = new_state;
        self.last_event_at = Some(now);
        true
    }
```

**Why `saturating_duration_since`:** if `now` is somehow earlier than `last` (clock skew across threads, although we won't have that in v0.1), `Instant::duration_since` panics. `saturating_duration_since` returns `Duration::ZERO`, which makes the comparison false (no debounce window has elapsed) and suppresses the transition — the safer default.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 41 passed; 0 failed`. (38 from Task 9 + 3 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(session): tracker debounces flapping transitions"
```

---

## Task 11: `AiStateTracker` — stale-state timeout (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs`

Spec: "stale-state timeout (default 30 s) that resets a session to `active` if a tool emits a state but never updates again — protects against stuck indicators when a tool dies mid-task." Implemented in `tick()`: if the current state isn't Active and `now - last_event_at >= config.stale_state`, reset to Active.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn tracker_stale_state_timeout_resets_to_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        assert_eq!(t.state(), TabState::Working);

        // 31 seconds later (past the 30 s default), tick → reset to Active.
        let later = now + Duration::from_secs(31);
        let changed = t.tick(later);
        assert!(changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_does_not_reset_within_stale_window() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        // 10 seconds later — still well within the 30 s stale window.
        let changed = t.tick(now + Duration::from_secs(10));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_stale_state_does_not_fire_when_already_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // Tracker starts Active; an Active->Active "transition" never sets
        // last_event_at, so tick() can't have a baseline. Should not fire.
        let changed = t.tick(now + Duration::from_secs(60));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_stale_state_after_idle_resets_to_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptStart),
            now,
        );
        assert_eq!(t.state(), TabState::Idle);
        // 31 seconds later — Idle should also be reset (stale-state spec
        // doesn't carve out shell-derived states).
        let changed = t.tick(now + Duration::from_secs(31));
        assert!(changed);
        assert_eq!(t.state(), TabState::Active);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 3 of the 4 new tests fail (the `does_not_reset_within_stale_window` test should already pass since `tick` is a no-op stub). Specifically: the timeout-firing tests will report `assert!(changed)` failures.

- [ ] **Step 2: Implement `tick` for stale-state**

Replace `fn tick` with:

```rust
    /// Stale-state and heuristic-silence checks at `now`. Returns `true` if a
    /// timeout caused a state change.
    pub fn tick(&mut self, now: Instant) -> bool {
        // Stale-state timeout: if we're not in Active and our last state-change
        // was more than `config.stale_state` ago, reset to Active.
        if self.state != TabState::Active {
            if let Some(last) = self.last_event_at {
                if now.saturating_duration_since(last) >= self.config.stale_state {
                    self.state = TabState::Active;
                    self.last_event_at = Some(now);
                    return true;
                }
            }
        }
        false
    }
```

Then remove the `#[allow(dead_code)] // first lib-level caller arrives in Task 11` annotation above `pub fn tick` (it's now reachable via the test in this task).

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 45 passed; 0 failed`. (41 from Task 10 + 4 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(session): tracker stale-state timeout resets to Active"
```

---

## Task 12: `AiStateTracker` — heuristic-silence timeout (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/tracker.rs`

Spec: "heuristic-output-silence timeout (default 4000 ms) used by Tier 3 fallback to infer `waiting` from observed quiet on a known AI process." Fires only when `heuristic_active = true`. Reads `last_output_at` (set by `TrackerInput::OutputObserved`) — if the tracker is currently Working and there's been no output for ≥ `config.heuristic_silence`, transition to Waiting.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn tracker_heuristic_silence_infers_waiting() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Working state set + last output observed.
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        t.on_input(TrackerInput::OutputObserved, now);

        // 5 seconds later — past the 4 s default heuristic_silence — but
        // BEFORE the debounce window from `now` would naturally have closed
        // (since 5 s > 100 ms). Heuristic timeout fires.
        let later = now + Duration::from_secs(5);
        let changed = t.tick(later);
        assert!(changed);
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_heuristic_silence_inactive_when_flag_off() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // heuristic_active stays false (default).
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        t.on_input(TrackerInput::OutputObserved, now);

        let changed = t.tick(now + Duration::from_secs(5));
        // No timeout fires; state stays Working until something else changes
        // it (or the stale-state timeout at 30 s).
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_heuristic_silence_does_not_fire_outside_working() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Tracker is Active (default). Heuristic only fires from Working.
        t.on_input(TrackerInput::OutputObserved, now);
        let changed = t.tick(now + Duration::from_secs(5));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_heuristic_silence_resets_on_new_output() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now,
        );
        // Output observed at now+3s — well within the 4 s silence window.
        t.on_input(
            TrackerInput::OutputObserved,
            now + Duration::from_secs(3),
        );
        // Tick at now+5s — only 2 s of silence since last output. No fire.
        let changed = t.tick(now + Duration::from_secs(5));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: 1 new test fails (`tracker_heuristic_silence_infers_waiting`). The other three pass because they assert "no change" and the current `tick` doesn't change anything in those cases.

- [ ] **Step 2: Add heuristic-silence check to `tick`**

Replace `fn tick` with:

```rust
    /// Stale-state and heuristic-silence checks at `now`. Returns `true` if a
    /// timeout caused a state change.
    pub fn tick(&mut self, now: Instant) -> bool {
        // Heuristic-silence (Tier 3): when active and Working, infer Waiting
        // after `config.heuristic_silence` of observed output silence.
        if self.heuristic_active && self.state == TabState::Working {
            if let Some(last_out) = self.last_output_at {
                if now.saturating_duration_since(last_out) >= self.config.heuristic_silence {
                    // Note: we bypass `transition_to` here because the heuristic
                    // is *itself* a debounce-tier signal — it shouldn't be
                    // suppressed by the 100 ms inter-transition window.
                    self.state = TabState::Waiting;
                    self.last_event_at = Some(now);
                    return true;
                }
            }
        }
        // Stale-state timeout: reset to Active if non-Active and inactive for
        // longer than `config.stale_state`.
        if self.state != TabState::Active {
            if let Some(last) = self.last_event_at {
                if now.saturating_duration_since(last) >= self.config.stale_state {
                    self.state = TabState::Active;
                    self.last_event_at = Some(now);
                    return true;
                }
            }
        }
        false
    }
```

Then remove the `#[allow(dead_code)] // first lib-level caller is in the App in Stage 3` line above `pub fn set_heuristic_active` — it's now called from the test in this task.

**Why heuristic-silence is checked first:** if both heuristic-silence and stale-state windows have elapsed (e.g. tracker was Working, no output, 31 s later we tick), we want the *more specific* signal — Waiting (inferred via heuristic) — not the more aggressive Active reset. Ordering matters.

**Why `transition_to` is bypassed:** the heuristic is the canonical signal that the tracker should react now. Running it through the debounce check would suppress correctness in the edge case where an `OutputObserved` arrived within 100 ms of the last AI frame and the heuristic timeout immediately follows — debounce would block it.

- [ ] **Step 3: Run tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 49 passed; 0 failed`. (45 from Task 11 + 4 new.)

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/tracker.rs
git commit -m "feat(session): tracker heuristic-silence timeout infers Waiting"
```

---

## Task 13: Integration test — dispatcher + tracker on a realistic stream

**Files:**
- Create: `crates/vibeflow/tests/integration.rs`

End-to-end test: feed a realistic mixed byte stream (shell prompt, command, AI tool output) through the dispatcher, route each event into the tracker, assert the state sequence we'd expect to see in a real terminal.

- [ ] **Step 1: Create the integration test file**

Write `crates/vibeflow/tests/integration.rs`:

```rust
//! Integration test: a realistic byte stream fed through `OscDispatcher` →
//! events routed into `AiStateTracker` → expected state sequence.

use std::time::{Duration, Instant};

use vibeflow::session::osc::{DispatchEvent, OscDispatcher, PromptMarker};
use vibeflow::session::tracker::{AiStateTracker, TabState, TrackerConfig, TrackerInput};

/// Helper: feed bytes, route events into the tracker, return a vector of
/// (post-event tracker state, was-it-a-change-bool) per event.
///
/// Each event is timestamped 200 ms after its predecessor in the same feed
/// call, well past the 100 ms debounce window. In real use, bytes arrive on a
/// PTY over wall-clock time so this models reality faithfully; the integration
/// test would otherwise see all events at the same instant and have legitimate
/// state transitions silently dropped by debounce.
fn feed_and_track(
    dispatcher: &mut OscDispatcher,
    tracker: &mut AiStateTracker,
    bytes: &[u8],
    start: Instant,
) -> Vec<(TabState, bool)> {
    let events = dispatcher.feed(bytes);
    events
        .into_iter()
        .enumerate()
        .map(|(i, ev)| {
            let now = start + Duration::from_millis(200 * i as u64);
            match ev {
                DispatchEvent::AiState(frame) => {
                    let changed = tracker.on_input(TrackerInput::AiFrame(frame), now);
                    (tracker.state(), changed)
                }
                DispatchEvent::Prompt(marker) => {
                    let changed = tracker.on_input(TrackerInput::Prompt(marker), now);
                    (tracker.state(), changed)
                }
                DispatchEvent::PassThrough(bytes) => {
                    // Real PTY/terminal-grid path (Stage 3+) would forward bytes
                    // here. For Stage 2, observing output through the tracker
                    // is the nearest equivalent.
                    let _ = bytes;
                    let changed = tracker.on_input(TrackerInput::OutputObserved, now);
                    (tracker.state(), changed)
                }
            }
        })
        .collect()
}

#[test]
fn shell_prompt_then_command_then_done() {
    // Wall-clock-style timestamps spread far enough apart to avoid debounce.
    let mut t0 = Instant::now();
    let mut bump = || {
        t0 += Duration::from_secs(1);
        t0
    };

    let mut d = OscDispatcher::new();
    let mut tr = AiStateTracker::new(TrackerConfig::default());

    // Shell renders a prompt: OSC 133;A then OSC 133;B, then prints "$ ".
    let prompt = b"\x1b]133;A\x07\x1b]133;B\x07$ ";
    let states = feed_and_track(&mut d, &mut tr, prompt, bump());
    // Two prompt markers + one passthrough ("$ "). The first PromptStart
    // transitions Active -> Idle; PromptEnd is Idle -> Idle (no change);
    // PassThrough drives OutputObserved (no change).
    assert_eq!(
        states,
        vec![
            (TabState::Idle, true),
            (TabState::Idle, false),
            (TabState::Idle, false),
        ]
    );

    // User runs `claude`. Shell emits OSC 133;C (command-start). Then claude
    // prints output, then emits OSC 1338;state=working, then more output, then
    // OSC 1338;state=waiting.
    let session = b"\x1b]133;C\x07hello from claude\
                   \x1b]1338;state=working;tool=claude\x07\
                   ...working...\
                   \x1b]1338;state=waiting;tool=claude\x07";
    let states = feed_and_track(&mut d, &mut tr, session, bump());

    // Expect, in order:
    //   - Prompt(CommandStart)             → Idle → Working (changed)
    //   - PassThrough("hello from claude") → OutputObserved (no change)
    //   - AiState(working)                 → Working (no change — same state)
    //   - PassThrough("...working...")     → OutputObserved (no change)
    //   - AiState(waiting)                 → Working → Waiting (changed)
    assert_eq!(
        states,
        vec![
            (TabState::Working, true),
            (TabState::Working, false),
            (TabState::Working, false),
            (TabState::Working, false),
            (TabState::Waiting, true),
        ]
    );

    // Claude exits. Shell emits OSC 133;D (command-end), then prints another
    // prompt.
    let after = b"\x1b]133;D;0\x07\x1b]133;A\x07$ ";
    let states = feed_and_track(&mut d, &mut tr, after, bump());

    // CommandEnd transitions Waiting → Idle (changed); PromptStart is Idle →
    // Idle (no change); PassThrough is OutputObserved.
    assert_eq!(
        states,
        vec![
            (TabState::Idle, true),
            (TabState::Idle, false),
            (TabState::Idle, false),
        ]
    );
}

#[test]
fn unknown_osc_passes_through_without_disturbing_tracker_state() {
    let mut d = OscDispatcher::new();
    let mut tr = AiStateTracker::new(TrackerConfig::default());
    let now = Instant::now();

    // Set tracker to Working via an explicit AI frame so we can observe that
    // an unknown OSC (window-title) doesn't change it.
    feed_and_track(&mut d, &mut tr, b"\x1b]1338;state=working\x07", now);
    assert_eq!(tr.state(), TabState::Working);

    // OSC 0 (window-title) is unknown — passes through, drives OutputObserved.
    let states = feed_and_track(
        &mut d,
        &mut tr,
        b"\x1b]0;my window title\x07",
        now + Duration::from_secs(1),
    );
    assert_eq!(states, vec![(TabState::Working, false)]);
    assert_eq!(tr.state(), TabState::Working);
}

#[test]
fn dispatcher_marker_smoke_check() {
    // A sanity check that the imports above resolve. Catches a refactor that
    // accidentally removes one of the public re-exports.
    let marker = PromptMarker::PromptStart;
    assert_eq!(format!("{marker:?}"), "PromptStart");
}
```

- [ ] **Step 2: Run the integration tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --test integration
```

Expected: `test result: ok. 3 passed; 0 failed`.

- [ ] **Step 3: Run the full test suite**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
```

Expected: `test result: ok. 49 passed; 0 failed` (unit) plus `test result: ok. 3 passed; 0 failed` (integration).

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: silent.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/tests/integration.rs
git commit -m "test(session): integration — dispatcher + tracker on realistic stream"
```

---

## Task 14: Final verification + tag

**Files:** none (verification + git tag)

- [ ] **Step 1: Full local CI dry-run**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo build --workspace --all-targets && \
  cargo test --workspace --all-targets && \
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps && \
  ( cd bindings/npm && npm run build && npm test ) && \
  echo "ALL GREEN"
```

Expected: trailing line is `ALL GREEN`. Anything else: stop, fix, re-run.

- [ ] **Step 2: 60-second fuzz check on the protocol parser**

Stage 1's fuzz target still applies — verify it still survives clean after Stage 2's workspace changes.

```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: "Done NN runs in 60 second(s)" with no crash output.

- [ ] **Step 3: Tag the milestone**

```bash
cd /home/bhengen/dev/vibeflow
git tag -a stage2-dispatcher-tracker-complete -m "OscDispatcher and AiStateTracker complete (Stage 2 of v0.1)"
git tag --list
```

(Don't push the tag yet — wait for the user.)

- [ ] **Step 4: Surface the result**

Report to the user:
- Number of new commits on this stage (should be 13–15 across Tasks 0–13).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 3 (PtySession + reader thread + headless App) as the next plan.

---

## Spec coverage check

Mapping spec sections → tasks in this plan:

| Spec section | Covered by |
|---|---|
| Components — `OscDispatcher` (`session/osc.rs`) | Tasks 1–6 |
| Components — `AiStateTracker` (`session/tracker.rs`) | Tasks 7–12 |
| Architecture — Bytes flow PTY → OscDispatcher → AiStateTracker | Task 13 (integration) |
| Data flow A — "Claude emits 'waiting'" | Task 13 first integration test (`shell_prompt_then_command_then_done`) |
| Data flow C — Plain shell, no AI | Task 13 first integration test (Prompt-driven Idle/Working) |
| Protocol — OSC 1338 (already in Stage 1) consumed by dispatcher | Tasks 3, 5 (recognised + reconstructed pass-through) |
| Protocol — OSC 133 prompt markers | Tasks 1, 4 |
| Tracker — `idle ↔ working ↔ waiting ↔ active` state machine | Tasks 7–9 (states); 10 (debounce); 11 (stale); 12 (heuristic) |
| Tracker — debounce default 100 ms | Task 10 |
| Tracker — heuristic-output-silence default 4000 ms | Task 12 |
| Tracker — stale-state default 30 s | Task 11 |
| Error handling — Malformed OSC 1338: ignore, restart parser at next ESC | Task 5 (`dispatcher_recovers_from_malformed_osc`) |
| Error handling — Truncated OSC 4 KiB cap | Task 5 (`dispatcher_drops_oversize_osc`) |
| Testing — Unit tests for OscDispatcher and AiStateTracker | All TDD tasks |
| Testing — `OscDispatcher::feed(arbitrary bytes)` never panics | Task 6 (in-tree proptest; cargo-fuzz target deferred to v0.2) |

**Out of scope for this plan (deferred to later stages, with rationale):**

- `PtySession` and reader threads — Stage 3.
- `App` glue (mpsc channels routing dispatcher events to tracker per session, plus heuristic-active toggle when foreground process changes) — Stage 3.
- `vibeflow` `[[bin]]` target — Stage 3 introduces `main.rs`.
- Cargo-fuzz target for `OscDispatcher::feed` (separate from in-tree proptest) — Stage 4+ once the spec has stabilised.
- Configuration parsing (`TrackerConfig` from TOML) — Stage 8.
- `aid=…` matching across `OSC 133;A`/`B` — not in v0.1 visual design; deferred to v0.2 if cmd-history features need it.

## Self-review

Performed by the plan author before saving:

- **Spec coverage:** every spec requirement above has a task. Three items (cargo-fuzz target, config parsing, aid matching) are explicitly out of scope and routed to later stages.
- **Placeholder scan:** no `TBD`, `TODO`, `implement later`, or "similar to" patterns. Each step has actual code or actual commands.
- **Type consistency check:**
  - `DispatchEvent` shape (Tasks 2, 3, 4, 5) consistent across all uses.
  - `PromptMarker` shape (Task 1) used identically in dispatcher (Task 4) and tracker (Task 9).
  - `TrackerInput` (Task 8) used identically in tracker (Tasks 9–12) and integration test (Task 13).
  - `TabState` (Task 7) used everywhere downstream.
  - `From<vibeflow_protocol::State>` (Task 7) used in `transition_to` indirectly via `frame.state.into()` in Tasks 8 and 9.
- **Clippy / fmt discipline:** every code-changing task ends with a `cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings` step before commit. Final Task 14 runs the workspace-wide CI dry-run.
- **Dead-code lifecycle:** two `#[allow(dead_code)]` attributes added in earlier tasks (`parse_133_body` in Task 1, `tick`/`set_heuristic_active` stubs in Task 8) are explicitly removed in their first-caller tasks (4, 11, 12 respectively).
