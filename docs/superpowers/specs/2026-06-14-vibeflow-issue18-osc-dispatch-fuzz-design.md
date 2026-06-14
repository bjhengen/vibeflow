# vibeflow #18 — Fuzz target for `OscDispatcher::feed` (split-frame reassembly)

**Date:** 2026-06-14
**Issue:** [#18](https://github.com/bjhengen/vibeflow/issues/18) — *Add a fuzz target for OscDispatcher::feed (split-frame reassembly)*
**Status:** design approved, pre-implementation

## Problem

The OSC 1338 frame parser (`vibeflow_protocol::parse`) has a libfuzzer target (`crates/vibeflow-protocol/fuzz/fuzz_targets/parse.rs`) and proptest round-trips. The **app-side streaming dispatcher** (`crates/vibeflow/src/session/osc.rs`, `OscDispatcher::feed`) — the stateful byte-stream layer that recognises OSC 1338 / OSC 133 / OSC 0/2 / OSC 52 across multiple `feed()` calls, enforces the `MAX_OSC_LEN` cap, and handles BEL-vs-ST termination — is only covered by unit tests and proptests. Its `Plain` / `SeenEsc` / `InOsc` / `InOscEsc` state machine and split-frame reassembly are exactly the kind of stateful logic a coverage-guided fuzzer is best at. Before the blog drives third-party eyes onto the protocol code, this layer should be fuzzed like the parser is.

## Goal

A second libfuzzer target that feeds arbitrary input through one `OscDispatcher` across many `feed()` calls, asserting:
1. **No panics** (regardless of input or split points), and
2. **The differential property:** feeding the input as arbitrary segments produces the same event stream as feeding it whole. This directly targets the reassembly logic — a streaming parser must be split-invariant.

## Non-goals

- Moving/refactoring `OscDispatcher` (it stays in the `vibeflow` app crate; see the rejected alternatives).
- Fuzzing anything beyond `OscDispatcher::feed`.
- Changing any production code.

## Design

### 1. Crate structure (hosting decision: app-crate fuzz crate)

`OscDispatcher` lives in the `vibeflow` app crate and is publicly reachable as `vibeflow::session::osc::OscDispatcher` (`pub mod session` in `lib.rs` → `pub mod osc` in `session/mod.rs` → `pub struct OscDispatcher` / `pub enum DispatchEvent`). The existing fuzz crate depends on `vibeflow-protocol`, which does not see the app crate.

Add a **new fuzz crate** `crates/vibeflow/fuzz/`, mirroring `crates/vibeflow-protocol/fuzz/`:

```toml
[workspace]

[package]
name = "vibeflow-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
arbitrary = { version = "1", features = ["derive"] }

[dependencies.vibeflow]
path = ".."

[[bin]]
name = "osc_dispatch"
path = "fuzz_targets/osc_dispatch.rs"
test = false
doc = false
bench = false
```

`DispatchEvent`/`OscDispatcher` depend only on `vibeflow_protocol` + std + sibling types in `osc.rs` (`PromptMarker`, `Osc52Selection`) — no GUI types — but they are compiled *as part of* the `vibeflow` crate, so the fuzz build pulls the app's full dependency graph (wgpu/winit). That is the accepted cost of not refactoring; it is a CI compile-time concern (mitigated by the Swatinem cache), not a correctness one.

**Rejected alternatives:** (B) move `osc.rs` into the published `vibeflow-protocol` crate — bloats the protocol crate's public API with terminal-emulator concerns (`SetTitle`/`Osc52Write`/`PassThrough` are not part of the OSC 1338 standard) and is a refactor with ripple; (B′) extract into a new internal `vibeflow-osc` crate — clean but disproportionate new-crate scaffolding for one fuzz target. Either could be a future cleanup if the fuzz-build weight becomes annoying.

### 2. The fuzz target (`crates/vibeflow/fuzz/fuzz_targets/osc_dispatch.rs`)

Typed input via `arbitrary`: the fuzzer supplies `Vec<Vec<u8>>` — the segments. This lets it explore segment boundaries directly (including empty segments and boundaries mid-escape-sequence), which is the reassembly surface we care about.

```rust
#![no_main]

use libfuzzer_sys::fuzz_target;
use vibeflow::session::osc::{DispatchEvent, OscDispatcher};

fuzz_target!(|segments: Vec<Vec<u8>>| {
    // Whole: feed the concatenation in one call.
    let whole_input: Vec<u8> = segments.concat();
    let mut whole_dispatcher = OscDispatcher::new();
    let whole_events = whole_dispatcher.feed(&whole_input);

    // Split: feed each segment through a fresh dispatcher, concatenating
    // the per-call event vectors.
    let mut split_dispatcher = OscDispatcher::new();
    let mut split_events = Vec::new();
    for seg in &segments {
        split_events.extend(split_dispatcher.feed(seg));
    }

    // Differential: a streaming parser must be split-invariant once the only
    // representation difference — how PassThrough byte-runs are chunked — is
    // normalised away.
    assert_eq!(
        coalesce_passthrough(whole_events),
        coalesce_passthrough(split_events),
        "OscDispatcher produced different events for whole vs segmented input"
    );
});

/// Merge consecutive `PassThrough(bytes)` events into one. `PassThrough` is the
/// only event whose representation legitimately varies with `feed()` chunking;
/// completed-sequence events (AiState/Prompt/SetTitle/Osc52Write) occur at the
/// same logical point regardless of split, so after coalescing the two streams
/// must be equal.
fn coalesce_passthrough(events: Vec<DispatchEvent>) -> Vec<DispatchEvent> {
    let mut out: Vec<DispatchEvent> = Vec::with_capacity(events.len());
    for ev in events {
        match (out.last_mut(), &ev) {
            (Some(DispatchEvent::PassThrough(acc)), DispatchEvent::PassThrough(next)) => {
                acc.extend_from_slice(next);
            }
            _ => out.push(ev),
        }
    }
    out
}
```

(`DispatchEvent` already derives `Debug, Clone, PartialEq, Eq`, so `assert_eq!` works.)

### 3. Soundness of the differential

The property "segmented feed ≡ whole feed (after PassThrough coalescing)" holds because the dispatcher buffers an in-progress OSC sequence in `osc_body` and only emits its event on the terminator — it never emits partial-sequence bytes as `PassThrough`. So a sequence split across `feed()` calls yields the same single completed event, just emitted on a later call; a dangling (unterminated) sequence at end-of-input emits nothing either way. The only chunk-dependent representation is runs of pass-through bytes, which `coalesce_passthrough` normalises.

**Verification step during implementation:** read `OscDispatcher::feed` and confirm no *other* output is split-dependent — in particular the `MAX_OSC_LEN` overflow-drop path (it should trigger on total accumulated length, deterministically, regardless of split). If a fuzz finding turns out to be a legitimate representation difference rather than a real bug, that means `coalesce_passthrough` is incomplete and is extended to normalise it away (normalise representation; never normalise away a genuine semantic difference). A real reassembly bug — a sequence recognised whole but missed when split, or vice versa — is exactly what this is meant to catch and must NOT be normalised away.

### 4. CI integration

Extend the existing `Fuzz smoke (60s)` job in `.github/workflows/ci.yml` (it already installs nightly + cargo-fuzz and caches `crates/vibeflow-protocol/fuzz`):
- Add a second Swatinem cache `workspaces:` entry for `crates/vibeflow/fuzz` (one cache step can list multiple workspaces).
- Add a run step after the parse fuzzer: `cargo +nightly fuzz run osc_dispatch -- -max_total_time=60`, `working-directory: crates/vibeflow`.

Heads-up: this step compiles the `vibeflow` dependency graph (wgpu/winit) under nightly + AddressSanitizer — slower than the protocol fuzz. The `Rust (stable)` job already builds that graph on the same runner image, so the system deps are present; the cost is build time, cached across runs.

## Testing / validation

- `cargo +nightly fuzz build osc_dispatch` (working-directory `crates/vibeflow`) compiles the target.
- `cargo +nightly fuzz run osc_dispatch -- -max_total_time=60` runs locally without a crash (the same smoke CI will run). A short run should reach the dispatcher quickly (the corpus starts empty; libfuzzer generates inputs).
- The existing `parse` fuzzer and the full `cargo test`/clippy/fmt gates remain green (this change adds files; it must not perturb the workspace build — note the fuzz crate has its own `[workspace]`, so `cargo test --workspace` from the repo root does NOT include it, matching the protocol fuzz crate).

## Risk / rollback

Additive: new fuzz crate + one CI step + CHANGELOG note. No production code touched. Rollback = revert the commit(s). App-only; ships in the next app release (v0.1.6) alongside #17. Worst realistic outcome of the work itself: the differential surfaces a real reassembly bug in `OscDispatcher` — in which case that becomes its own fix (a *good* outcome for a pre-blog hardening pass), tracked separately rather than expanded into this change.

## Files

- Create: `crates/vibeflow/fuzz/Cargo.toml`
- Create: `crates/vibeflow/fuzz/fuzz_targets/osc_dispatch.rs`
- Modify: `.github/workflows/ci.yml` (fuzz job: cache workspace + run step)
- Modify: `CHANGELOG.md` (`[Unreleased]` note)
- (cargo-fuzz also generates `crates/vibeflow/fuzz/.gitignore` + `Cargo.lock` on first build; commit per the protocol fuzz crate's existing convention.)
