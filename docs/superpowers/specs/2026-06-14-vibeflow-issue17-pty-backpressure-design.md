# vibeflow #17 — Bound the PTY reader channel (backpressure)

**Date:** 2026-06-14
**Issue:** [#17](https://github.com/bjhengen/vibeflow/issues/17) — *PTY reader channel is unbounded — add backpressure for sustained output firehose*
**Status:** design approved, pre-implementation

## Problem

`PtySession::spawn` (`crates/vibeflow/src/session/session.rs`) wires the reader thread to the main loop through an **unbounded** `std::sync::mpsc::channel::<Vec<u8>>`. The reader thread reads 4 KiB chunks from the PTY master and `send`s them as fast as the PTY produces.

The v0.1.4 throughput work (#10) bounds per-poll *processing* (`MAX_POLL_BYTES` = 64 KiB per `poll()`, with `output_pending` immediate re-wake) so the UI stays live during bursts — but it does **not** bound the *queue*. Under a sustained firehose (`cat /dev/urandom`, `cat /dev/zero`, `yes`, `seq 1 50000000`, a runaway agent dumping gigabytes) the reader produces at PTY speed (hundreds of MB/s) while the main loop consumes at parse speed (~9 MB/s). The channel buffers the difference as unbounded heap growth → memory blow-up / OOM. This is a trivially reproducible "I piped a big file into it and it ate all my RAM" failure — the kind of thing a curious reader will hit immediately once vibeflow gets attention.

## Goal

Bound the queue so a fast producer is throttled by the slow consumer, exactly the way terminals have always throttled fast producers: the kernel PTY buffer fills, the child's `write()` blocks, the child slows down. No bytes are dropped (correct terminal semantics — backpressure, not loss).

## Non-goals

- Changing the per-poll processing budget (#10 `MAX_POLL_BYTES`) — unchanged.
- VT parse batching / dirty-region rendering / OscDispatcher double-pass — separate deferred items.
- Any change to steady-state throughput.

## Design

### 1. Bounded channel

In `PtySession::spawn`, replace

```rust
let (tx, rx) = mpsc::channel::<Vec<u8>>();
```

with a **bounded synchronous channel**:

```rust
const READER_CHANNEL_CAPACITY: usize = 512; // chunks; 4 KiB each ≈ 2 MiB/tab
let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(READER_CHANNEL_CAPACITY);
```

The reader thread's `tx.send(chunk)` now **blocks** when 512 chunks are queued instead of allocating. The blocked send is the backpressure: the PTY master's kernel buffer fills, and the child's next `write()` blocks. The existing `send(...).is_err() → break` path (receiver dropped) is unchanged — `SyncSender::send` returns the same `Result`/`SendError` on a dropped receiver.

**Capacity rationale:** 512 chunks ≈ 2 MiB worst-case buffered per tab — 32× the 64 KiB (`MAX_POLL_BYTES` = 16-chunk) poll budget. The large multiple guarantees a `poll()` can always drain a full 64 KiB budget without the reader idling on a backpressured send, so steady-state throughput is unchanged; only a *sustained* multi-MB/s firehose ever reaches the limit. Bursty-but-bounded output (build logs, `ls -R`, a few-MB dump) never blocks.

### 2. Deadlock-free teardown (the careful part)

Today the reader thread is normally blocked in `reader.read()` (PTY master). `Drop` does `child.kill()` → the PTY hits EOF → `read()` returns `Ok(0)` → the thread breaks → `join()` returns.

With a **bounded** channel the reader can instead be blocked in `tx.send()` (queue full). `child.kill()` does **not** unblock a blocked `send()` — so the existing `Drop` (`child.kill()` then `reader_thread.join()`) would **hang forever** whenever a tab is closed while its queue is full (precisely the firehose case). This is the load-bearing correctness requirement of this change.

**Fix:** make the receiver droppable from `Drop`, and drop it *before* joining. Dropping the `Receiver` makes any blocked `SyncSender::send` return `Err(SendError)` immediately → the reader breaks → `join()` returns.

- Change the field `rx: Receiver<Vec<u8>>` → `rx: Option<Receiver<Vec<u8>>>`.
- `spawn` stores `rx: Some(rx)`.
- `poll()` reads through the `Option`. It is only ever `None` during teardown, never during a live poll; use `let Some(rx) = self.rx.as_ref() else { return events; };` (or equivalent) at the top of the drain loop.
- `Drop`:
  ```rust
  fn drop(&mut self) {
      let _ = self.child.kill();
      // Drop the receiver BEFORE join: a reader blocked on a full-channel
      // send() only wakes when the receiver goes away (send → Err) — kill()
      // alone can't unblock it. Without this, closing a tab mid-firehose
      // deadlocks on join(). (#17)
      self.rx = None;
      if let Some(handle) = self.reader_thread.take() {
          let _ = handle.join();
      }
  }
  ```

`respawn` replaces `*self` with a fresh session, which drops the old value and runs this `Drop` — so it is covered with no extra code.

**Approach chosen:** drop-the-receiver-before-join (above). Considered and rejected: (B) *drain-then-join* (`while self.rx.try_recv().is_ok() {}` before join) — works only because `kill()` stops production, subtler and races a still-producing child; (C) *detach / don't join* — leaks the reader thread on every tab close until it happens to wake, abandons today's clean teardown.

### 3. Throughput

Unchanged by construction (capacity ≫ poll budget — see §1). Re-run the existing `perf_probe_parse_drain_throughput` test (`seq 1 2000000`, ~16.9 MB) before/after and confirm the reported MB/s is not materially lower (~9 MB/s baseline).

## Testing

1. **New regression test — deadlock-free teardown under a full channel (the important one).**
   Spawn a firehose child (e.g. `["/bin/sh", "-c", "cat /dev/zero"]` or `yes`), `poll()` a few times to start the child, then **stop polling** so the bounded channel fills and the reader blocks on `send()`. Drop the session and assert `Drop` returns within a short deadline. Implementation: move the session into a worker thread that drops it, and assert the worker `join`s within e.g. 5 s (a hang = failure). This both guards the deadlock and implicitly proves the bound engages (an unbounded channel never blocks the sender, so the reader could never be the thing we need to unblock). Uses a real PTY → must be serial-safe (`--test-threads=1`; see the subprocess-flakiness lesson).

2. **Keep:** `perf_probe_parse_drain_throughput` (throughput unchanged).

Optional (not planned): a direct "child blocks under backpressure" assertion — redundant with test 1 and timing-flaky; skip.

## Risk / rollback

Single-file change (`session.rs`) plus the test. Rollback is reverting the commit. The only behavior change visible to users is the intended one: a runaway child is throttled instead of growing memory unbounded. No protocol, config, or rendering change → app-only; ships in the next app release (v0.1.6, bundled with #18).

## Files

- `crates/vibeflow/src/session/session.rs` — channel construction, `rx` field → `Option`, `poll()` access, `Drop`, new test.
