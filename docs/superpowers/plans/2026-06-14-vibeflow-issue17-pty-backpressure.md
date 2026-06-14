# PTY Reader Channel Backpressure (#17) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound the PTY reader→main-loop channel so a runaway child is throttled by backpressure instead of growing heap unbounded, with deadlock-free teardown.

**Architecture:** Replace the unbounded `mpsc::channel` with `mpsc::sync_channel(512)` (the reader thread blocks on `send` when full → kernel PTY buffer fills → child `write()` blocks). Because the reader can now be blocked in `send()` instead of `read()`, `Drop` must drop the receiver *before* `join()` (a blocked `send` only wakes when the receiver goes away). The receiver field becomes `Option<Receiver<…>>` so `Drop` can take it.

**Tech Stack:** Rust, `std::sync::mpsc::sync_channel`, existing `PtySession` in `crates/vibeflow/src/session/session.rs`.

**Spec:** `docs/superpowers/specs/2026-06-14-vibeflow-issue17-pty-backpressure-design.md`

---

### Task 1: Bound the channel and make teardown deadlock-free

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs` (capacity const; `spawn` channel + `rx` init; `rx` struct field → `Option`; `poll` access; `Drop`; two existing tests that read `rx` directly)
- Test: `crates/vibeflow/src/session/session.rs` (new regression test in the existing `#[cfg(test)] mod tests`)

This task is TDD red→green within one commit: steps 1–3 introduce the bounded channel and a regression test that **fails** (teardown deadlocks), steps 4–5 apply the teardown fix that makes it **pass**. Commit only once green (step 7) — no broken commit lands.

- [ ] **Step 1: Introduce the bounded channel (no teardown fix yet)**

Add the capacity constant next to `MAX_POLL_BYTES` (currently `crates/vibeflow/src/session/session.rs:32`):

```rust
/// #17: bounded reader→main-loop channel capacity, in 4 KiB chunks. The reader
/// thread blocks on `send` once this many chunks are queued, applying
/// backpressure (the PTY kernel buffer fills, the child's writes block) instead
/// of buffering a firehose as unbounded heap. 512 × 4 KiB ≈ 2 MiB/tab, 32× the
/// `MAX_POLL_BYTES` drain budget, so steady-state throughput is unchanged.
const READER_CHANNEL_CAPACITY: usize = 512;
```

In `PtySession::spawn`, change the channel construction (currently `crates/vibeflow/src/session/session.rs:243`):

```rust
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(READER_CHANNEL_CAPACITY);
```

(was `let (tx, rx) = mpsc::channel::<Vec<u8>>();`). The reader thread's `tx.send(...)` call site is unchanged — `SyncSender::send` has the same signature and returns `Err` on a dropped receiver, so the existing `if tx.send(...).is_err() { break; }` path still compiles and behaves the same. Leave the `rx` struct field, `poll`, and `Drop` untouched for now.

- [ ] **Step 2: Write the failing regression test**

Add to the `#[cfg(test)] mod tests` block in `crates/vibeflow/src/session/session.rs` (it already has `use super::*;`, `use std::time::{Duration, Instant};`, the `wait_until` helper, and uses `TrackerConfig::default()`):

```rust
    #[test]
    fn drop_does_not_hang_when_reader_blocked_on_full_channel() {
        // #17: with the bounded channel the reader thread can be blocked in
        // send() (queue full) rather than read(). Drop must drop the receiver
        // before join() so the blocked send returns Err and the thread exits —
        // otherwise closing a tab mid-firehose deadlocks on join().
        let s = PtySession::spawn(
            &["/bin/sh", "-c", "cat /dev/zero"],
            TrackerConfig::default(),
            10000,
        )
        .unwrap();
        // Do NOT poll: let the reader fill the 512-chunk channel (~2 MiB, fills
        // in milliseconds) and block on send().
        std::thread::sleep(Duration::from_millis(300));
        // Drop on a worker thread; the main thread asserts it completes promptly.
        let dropper = std::thread::spawn(move || drop(s));
        let finished = wait_until(Duration::from_secs(5), || dropper.is_finished());
        assert!(
            finished,
            "PtySession::drop() hung — a reader blocked on a full channel was \
             not unblocked at teardown (#17 regression)"
        );
        dropper.join().unwrap();
    }
```

- [ ] **Step 3: Run the test and verify it FAILS (deadlock)**

Run: `cargo test -p vibeflow --lib drop_does_not_hang_when_reader_blocked_on_full_channel -- --test-threads=1`
Expected: **FAIL** — the assert fires after ~5 s because `Drop` (`child.kill()` then `join()`) hangs: the reader is blocked in `send()`, the still-alive receiver never lets it return `Err`, and `kill()` cannot unblock a blocked `send`. This demonstrates the deadlock the bounded channel introduces.

- [ ] **Step 4: Apply the teardown fix**

(a) Change the `rx` struct field (currently `crates/vibeflow/src/session/session.rs:137`):

```rust
    /// Drains here when the reader thread sends bytes from the PTY master.
    /// `Option` so `Drop` can drop the receiver before joining the reader
    /// thread — a reader blocked on a full `sync_channel` send only wakes when
    /// the receiver goes away (#17).
    rx: Option<Receiver<Vec<u8>>>,
```

(b) In `spawn`'s returned struct literal, change `rx,` to:

```rust
            rx: Some(rx),
```

(c) In `poll`, change the receive site. Replace (currently `crates/vibeflow/src/session/session.rs:367`):

```rust
            match self.rx.try_recv() {
                Ok(chunk) => {
```

with:

```rust
            let recv = match self.rx.as_ref() {
                Some(rx) => rx.try_recv(),
                None => break, // receiver taken during teardown; nothing to drain
            };
            match recv {
                Ok(chunk) => {
```

The two `Err(mpsc::TryRecvError::Empty) => break,` / `Err(mpsc::TryRecvError::Disconnected) => { … }` arms now match on `recv` and are otherwise unchanged. (The per-iteration `self.rx.as_ref()` borrow ends as soon as `try_recv()` returns its owned `Result`, so it does not conflict with the `&mut self` calls in the loop body, e.g. `self.refresh_default_subtitle()`.)

(d) Change `Drop` (currently `crates/vibeflow/src/session/session.rs:795`):

```rust
impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // #17: drop the receiver BEFORE join. A reader blocked on a full
        // sync_channel send() only wakes when the receiver is gone (send →
        // Err); kill() alone can't unblock it. Without this, closing a tab
        // mid-firehose deadlocks here on join().
        self.rx = None;
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}
```

(e) Fix the two EXISTING tests that read the `rx` field directly — they call `recv_timeout` on the bare `Receiver` and won't compile once it's an `Option`. Both sites are the identical line `match s.rx.recv_timeout(Duration::from_millis(100)) {` (currently `crates/vibeflow/src/session/session.rs:1011` in `send_input_round_trips_bytes_through_pty`, and `:1133` in `session_reader_thread_pumps_bytes_to_channel`). Change **both** to:

```rust
            match s.rx.as_ref().unwrap().recv_timeout(Duration::from_millis(100)) {
```

(`unwrap()` is sound in these tests — `rx` is only `None` during `Drop`, never during the test body.)

- [ ] **Step 5: Run the test and verify it PASSES**

Run: `cargo test -p vibeflow --lib drop_does_not_hang_when_reader_blocked_on_full_channel -- --test-threads=1`
Expected: **PASS** — dropping the receiver makes the blocked `send` return `Err`, the reader breaks, `join()` returns, and the dropper thread finishes well within 5 s.

- [ ] **Step 6: Full gates (suite + clippy + fmt)**

Run: `cargo test -p vibeflow --lib -- --test-threads=1`
Expected: PASS (all lib tests; serial per the subprocess-flakiness convention).

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean (exit 0).

Run: `cargo fmt --check`
Expected: clean (exit 0).

- [ ] **Step 7: Commit**

```bash
git add crates/vibeflow/src/session/session.rs
git commit -m "$(cat <<'MSG'
fix(#17): bound PTY reader channel with backpressure + deadlock-free teardown

Replace the unbounded reader->main-loop mpsc::channel with sync_channel(512)
(~2 MiB/tab, 32x the 64 KiB poll budget). The reader thread now blocks on send
when the queue is full -> the PTY kernel buffer fills -> the child's writes
block: a runaway firehose (cat /dev/zero, yes, multi-GB dumps) is throttled
instead of growing heap unbounded. No bytes dropped (correct terminal
backpressure semantics).

Because the reader can now be blocked in send() rather than read(), Drop drops
the receiver before join() (rx is now Option) — a blocked send only wakes when
the receiver goes away; kill() alone can't unblock it. Without this, closing a
tab mid-firehose deadlocks on join(). New regression test asserts Drop returns
promptly with the channel full.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 2: Validate throughput and update the changelog

**Files:**
- Modify: `CHANGELOG.md` (`[Unreleased]`)

- [ ] **Step 1: Re-validate steady-state throughput**

Run the existing throughput probe (it prints MB/s; it is not an assertion). It is `#[ignore = "perf probe"]`, so `--ignored` is required:
`cargo test -p vibeflow --lib perf_probe_parse_drain_throughput -- --ignored --test-threads=1 --nocapture`
Expected: the printed `=> ~N MB/s` is not materially below the ~9 MB/s baseline (capacity 512 ≫ the 16-chunk poll budget, so the reader never starves a drain). Record the number in the commit message. If it *is* materially lower, stop — the capacity/teardown interaction needs review before proceeding.

- [ ] **Step 2: Add the CHANGELOG entry**

Under `## [Unreleased]` in `CHANGELOG.md`, add (create a `### Fixed` subsection if one isn't already present under Unreleased):

```markdown
### Fixed

- **PTY reader channel is now bounded (#17).** The reader thread → main-loop channel was an unbounded `mpsc::channel`; a sustained output firehose (`cat /dev/zero`, `yes`, a runaway agent dumping gigabytes) could buffer unbounded heap between polls (reader produces at hundreds of MB/s, parser drains at ~9 MB/s). It is now a `sync_channel(512)` (~2 MiB/tab): the reader blocks on a full queue, the PTY kernel buffer fills, and the child's writes block — backpressure, no bytes dropped. Teardown drops the receiver before joining the reader thread so closing a tab mid-firehose can't deadlock. Steady-state throughput is unchanged.
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(#17): changelog entry for bounded PTY reader channel"
```

---

## Self-Review

- **Spec coverage:** §1 bounded channel → Task 1 steps 1, 7. §2 deadlock-free teardown (rx→Option, drop-before-join) → Task 1 step 4. §3 throughput → Task 2 step 1. Testing (deadlock regression + perf revalidation) → Task 1 steps 2–5, Task 2 step 1. CHANGELOG/app-only release note → Task 2. All covered.
- **Placeholder scan:** none — every code/diff/command is concrete.
- **Type consistency:** `READER_CHANNEL_CAPACITY` (const), `rx: Option<Receiver<Vec<u8>>>`, `self.rx.as_ref()` in `poll`, `self.rx = None` in `Drop`, `rx: Some(rx)` in `spawn`, test name `drop_does_not_hang_when_reader_blocked_on_full_channel` — consistent across tasks. `mpsc::sync_channel` returns `(SyncSender, Receiver)`; `tx.send` and `try_recv` call sites unchanged.
- **Senior review (2026-06-14, Sonnet) applied:** caught one compile blocker — two existing tests (`send_input_round_trips_bytes_through_pty`, `session_reader_thread_pumps_bytes_to_channel`) read `s.rx.recv_timeout(...)` on the bare `Receiver`; now updated in Step 4(e) to `s.rx.as_ref().unwrap().recv_timeout(...)`. Verified there are no other `rx` field accesses. All other claims (borrow-checker safety of the per-iteration `as_ref()`, `sync_channel`/`SyncSender::send`/`JoinHandle::is_finished` API, line numbers, TDD red genuinely deadlocks, `perf_probe` is `#[ignore]`) confirmed against source.
