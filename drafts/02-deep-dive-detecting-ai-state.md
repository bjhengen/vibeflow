# How vibeflow knows what your AI agent is doing

> **Draft — deep-dive post.** This is the one to submit to Hacker News (submit the blog URL;
> discussion happens in the HN comments). ~1,800 words.

*vibeflow is an open-source Linux terminal that shows, per tab, whether the program inside is
working or waiting on you. This is how the detection works — the open protocol, the heuristic
fallback, and the parts that were harder than they looked.*

A terminal emulator renders a grid of characters coming out of a pty. It has no model of what
the program on the other end is *doing* — and historically it didn't need one, because a human
was typing every command and already knew. AI coding agents break that assumption. They run for
minutes, then stop and wait for input, and from the terminal's point of view "thinking hard" and
"waiting for you" are the same thing: a quiet pty.

I wanted vibeflow to close that gap — to show, per tab, whether the thing inside is working or
waiting. The design question is the interesting part: **how do you make a terminal aware of
program state without (a) hard-coding knowledge of specific tools, or (b) requiring every tool
to cooperate before the feature does anything useful?**

None of this is unprecedented, and it's worth saying so up front. Shell-integration sequences —
FinalTerm's original OSC 133, iTerm2's extensions, the prompt markers many shells now emit —
already taught terminals to recognize where a prompt begins and a command ends. tmux can shell
out to `notify-send`. What I haven't seen is a small, *open* protocol specifically for **agent
state**, paired with a fallback that works when the tool emits nothing at all. That pairing is
vibeflow's answer, and it has two layers.

## Layer 1: an open protocol (OSC 1338)

The high-fidelity path is for the tool to just tell you. The cleanest channel for that is the
one already connecting the tool to the terminal: the pty byte stream.

OSC ("Operating System Command") escape sequences are the established way for a program to send
out-of-band metadata to its terminal — setting the window title (OSC 0/2), writing the clipboard
(OSC 52), and so on. Terminals that don't understand a given OSC ignore it. That property is
exactly what you want for an opt-in feature: a tool can emit state unconditionally, and on any
other terminal the bytes simply vanish. No capability negotiation, no breakage.

So vibeflow defines one:

```
ESC ] 1338 ; key=value [ ; key=value ]* ( BEL | ST )
```

The frame that matters most looks like this on the wire:

```
\x1b]1338;state=waiting;tool=claude;project=vibeflow\x07
```

The grammar is deliberately tiny:

- **`state`** (required) is one of `active`, `working`, `waiting`, `done`.
- **`tool`** (optional) names the emitter — `claude`, `codex` — for display and grouping.
- **`project`** (optional) surfaces in the tab's subtitle.

Values are percent-encoded with uppercase hex if they'd otherwise collide with the `;` / `=`
delimiters or contain control or non-ASCII bytes, so `a;b=c` rides across as `a%3Bb%3Dc`. Frames
are capped at 4 KiB; anything longer is dropped on the floor rather than parsed.

Why the number 1338? It's unclaimed by the common sequences, and the protocol *owns* it: it
means nothing to any other terminal, and the stability rule is simple — additive changes (new
keys, new state values) stay safe for old parsers, and a genuinely breaking change would bump
the identifier itself. (And yes, it lands one past iTerm2's OSC 1337. Make of that what you
will.)

Emitting is meant to be a one-liner. The protocol ships as a Rust crate:

```rust
use vibeflow_protocol::{emit, Frame, State};

emit(&Frame::new(State::Waiting)
    .with_tool("claude")
    .with_project("vibeflow"))?;
```

…a CLI for shell scripts and hooks:

```
vibeflow-emit waiting --tool=claude
```

…and an npm package (`vibeflow-protocol`) for Node-based tools. The receiving end calls the same
`parse()` from the same crate, so emitter and consumer can't drift apart.

### The /dev/tty detour

Here's the first thing that didn't work the obvious way. To make Claude Code emit these, you
hang `vibeflow-emit` off its hooks. The natural implementation writes the escape sequence to
stdout — that's where terminal output goes.

Except Claude Code runs hook commands with their stdout *captured* — it uses hook output for its
own purposes. So the bytes I wrote to stdout never reached the pty, and the tab never lit up. The
fix is to write directly to the controlling terminal: `vibeflow-emit` opens `/dev/tty` and writes
the sequence there, which lands on the real pty regardless of how stdout is redirected. (There's
an env override, `VIBEFLOW_EMIT_STDOUT=1`, for the pipe-it-yourself case.) Small thing, but it's
the difference between the feature working and silently doing nothing.

### Why five hooks

The other surprise was hook *coverage*. You'd think two hooks would do it — one for "started,"
one for "stopped." In practice the Claude Code wiring needs five:

```jsonc
UserPromptSubmit  → working
PreToolUse        → working
PostToolUse       → working
Stop              → waiting
Notification      → waiting
```

The reason is that Claude Code fires `Stop` at the end of *every* response, including the brief
pauses between tool-call rounds inside a single turn. Wire up only `Stop` / `UserPromptSubmit`
and the tab flickers amber every time the agent pauses to run a tool mid-turn. Covering
`PreToolUse` / `PostToolUse` with `working` holds the state steady through those internal
transitions, so amber means what you want it to mean: the turn is actually over and it's your
move. This is a quirk of the tool's lifecycle, not the protocol — but it's exactly the kind of
thing you only learn by watching the stripe misbehave.

## Layer 2: the heuristic fallback

A protocol only helps for tools that adopt it. I didn't want vibeflow to be useless out of the
box, or to require everyone to wire up hooks before they saw any value. So there's a fallback
that needs zero cooperation from the tool.

vibeflow thinks in three tiers, in priority order:

1. **Tier 1 — native OSC 1338.** The tool emits its own state. Authoritative.
2. **Tier 2 — wrapper shims.** A drop-in launcher (think `vibeflow-claude`) that watches a
   tool's output and emits on its behalf. Planned, not yet shipped.
3. **Tier 3 — a `/proc` heuristic.** vibeflow infers state with no help from the tool at all.
   This is what gives you a useful stripe on day one.

Tier 3 works like this. For each tab, vibeflow already knows the pty's child pid. On Linux it
reads `/proc/<pid>/stat`, pulls the `tpgid` field — the foreground process group of the
controlling terminal — and then reads `/proc/<tpgid>/comm` to get the name of whatever's actually
in the foreground right now. If that name is in your configured AI-tool list
(`[ai] tools = ["claude", "codex", …]` in the TOML config), the tab is "heuristic-armed." This is
polled on a throttle, roughly every 250 ms.

From there it's a small state machine driven by output and time:

- Any output from an armed tab → **Working**, and the silence timer resets.
- **4 seconds** of silence while Working → **Waiting**. That's the inference: a known agent that
  was producing output and then went quiet has almost certainly handed the turn back to you.

The timing constants are deliberately boring and tunable: a 100 ms debounce so rapid transitions
don't thrash, the 4 s silence window, and a 30 s stale-state timeout that resets a tab to neutral
after long inactivity.

## The hard part: not lying to the user

The mechanism above is easy. Making it *trustworthy* is where the real work was, and it comes
down to a few rules that all exist to keep the indicator from ever asserting something false.

**Explicit always wins, permanently.** The instant a tab receives even one real OSC 1338 frame,
vibeflow stops running the heuristic for that tab — for the rest of the session. A tool that
speaks the protocol knows its own state far better than my silence timer ever could, and the
worst outcome is the two fighting each other. So the heuristic loses its vote the moment the
authoritative signal shows up.

**Waiting persists; everything else decays.** Most states are transient and should quietly
reset — a `Working` tab that's been silent for 30 seconds with nothing else going on should fade
to neutral rather than keep claiming it's working. But `Waiting` is the headline state, the whole
reason the tool exists, and it means "needs you, still unacknowledged." So `Waiting` is explicitly
*exempt* from the stale-state reset: amber stays amber until you actually go act on it or the tool
moves on. For explicitly-emitting tools there's a parallel 5-minute fuse that de-escalates a stuck
`Working` back to neutral but, again, leaves `Waiting` alone.

**The sharp edge I chose to keep.** This persistence has a consequence worth being honest about.
If an agent finishes (amber) and you then drop to a bare shell in that tab without ever
acknowledging it, the tab can *stay* amber — because a plain shell with no prompt-marker
integration emits no signal that would clear it. That looks like a stuck indicator. It's actually
the intended semantics: "you were needed here and haven't dealt with it yet." Enabling OSC 133
prompt markers in your shell, or just running the next thing, clears it. I went back and forth and
decided a persistent "you still haven't looked at this" was more useful than an amber that times
out and lets you miss the thing entirely.

## The limits, stated plainly

The heuristic is a heuristic. Tier 3 is Linux-only, because it leans on `/proc`. The `comm` field
the kernel exposes is truncated to 15 characters, so a tool whose process name is longer than that
will never match the configured list (a real gotcha that cost me some confusion). And "silence
means waiting" genuinely isn't always true — a long compile or a slow network call inside an armed
tool reads as `Waiting` even though nobody's needed. That's the price of inferring without
cooperation, and it's exactly why Tier 1 exists: the protocol turns a good guess into a fact.

## The rendering side, briefly

Once the state is known, drawing it is the easy half. Each tab gets a 6-pixel stripe down its left
edge, color-coded — blue for working, amber for waiting, green for a just-finished `done`, gray for
idle, nothing for a plain active command. The one bit of motion in the whole UI is reserved for the
state that earns it: `Waiting` pulses, on a 1.4-second sine cycle between 40% and 100% opacity.
Working is a steady stripe; only "needs you" moves. The restraint is deliberate — if everything
pulsed, nothing would.

> 📸 **[SCREENSHOT (optional but nice): close-up of the tab bar showing the stripe colors —
> working/blue, waiting/amber, idle/gray side by side.]**

(One small modeling note: the protocol's vocabulary is four states, but the terminal's internal
notion of a tab adds `Idle` — a shell sitting at a prompt with nothing running — which only
vibeflow itself assigns. A tool can't emit `idle`, because only the terminal is in a position to
know it.)

## Where this goes

The terminal is fun to build, but the part I actually care about is the protocol. OSC 1338 is a
small, documented, MIT/Apache-2.0 thing, and it's far more useful if it isn't just vibeflow's
private handshake. If you build terminals, or AI coding tools, I'd love for you to emit it, consume
it, or tell me where the design is wrong. The spec and the crates are on GitHub.

For full honesty: I built vibeflow solo, I'm newish to Rust, and a lot of it was written with
Claude Code — which is either a fitting origin story for a tool aimed at AI-assisted coding or a
reason to read the source skeptically. Both, probably. Either way it's open; come look.

- Repo & protocol spec: <https://github.com/bjhengen/vibeflow>
- `cargo install vibeflow`, or the AppImage on the releases page.
