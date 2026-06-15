# Addendum: hardening since the first draft (v0.1.6)

> **Draft — addendum.** Intended to splice into the deep-dive
> (`02-deep-dive-detecting-ai-state.md`), most naturally after **"The limits,
> stated plainly"** or **"The rendering side, briefly."** Standalone so it
> doesn't collide with edits to the main posts. Cut/condense freely. ~500 words.

I wrote the posts above against v0.1.4. In the weeks since I've been running vibeflow as my daily driver, and three problems surfaced that are worth telling honestly — partly because they're the kind of thing the "render glyphs vs. understand state" framing predicts, and partly because the bugs themselves were good ones.

## The pulsing amber tab flickers over VNC

The money-shot — a gently pulsing amber "waiting" stripe — has a caveat I didn't know when I drafted this: over VNC (or any remote/software X server) it can make the whole screen flicker, and it gets *worse the more you use it*.

That "worse with use" smell screamed memory leak. It wasn't. Resident memory was flat for hours; the GPU held a steady 33 MiB. The actual cause is more interesting: vibeflow has no damage tracking yet, so every repaint re-presents the **entire** surface. The amber pulse is a 1.4-second sine animation, so a waiting tab repaints ~10×/second forever — to animate a six-pixel stripe. On a real GPU that full-surface present is free and invisible. On a software X server like TigerVNC, each present is re-encoded as *full-screen* damage and streamed to the client. At ten of those a second, multiplied by however many tabs are waiting, you get flicker. The final tell: it tracked my **GPU load**, not uptime — when a local LLM was hammering the card, the VNC re-encode couldn't keep up and the flashing appeared; idle, the identical presents were invisible.

The honest fix shipped as an opt-out: `[ui] indicator_pulse = false` renders a steady amber stripe instead of animating one, so a waiting tab stops driving presents. Native displays keep the pulse by default. The *real* fix — present only the changed rectangle — needs damage-aware presentation, which the safe wgpu surface API doesn't currently expose; that's tracked, not done. A good reminder that "the terminal re-sends the whole screen to animate a stripe" is exactly the sort of waste you stop noticing on fast hardware.

## A firehose could eat all your RAM

The thread that reads a tab's pty handed bytes to the main loop through an *unbounded* channel. Pipe something relentless — `cat /dev/zero`, a runaway agent dumping gigabytes — and the reader produces at hundreds of MB/s while the parser drains at ~9 MB/s. The difference piles up as unbounded heap. It's a ten-second way to OOM the process, and precisely the report a curious stranger files first.

It's now a bounded channel: the reader blocks when the queue is full, the kernel pty buffer fills, and the child's writes block — backpressure, the way terminals have throttled fast producers forever, no bytes dropped. The fiddly part was teardown: a reader blocked on a full channel can't be woken by killing the child, so closing a tab mid-firehose would deadlock unless you drop the receiver before joining the thread.

## Fuzzing the part this post is about

The streaming dispatcher that recognizes OSC 1338 across arbitrarily-split reads — the reassembly described above — now has a differential fuzzer: feed the same bytes as random segments and as one chunk, and the event streams must match. It's the property a streaming parser has to satisfy, and it ran a million-plus iterations clean. If you're going to write a post claiming your parser handles split frames correctly, it's worth having a machine try to prove you wrong first.
