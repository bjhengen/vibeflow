# I built a terminal that knows when your AI is waiting on you

> **Draft — overview post.** Canonical home: personal blog. Reuse for Medium (cross-post)
> and a condensed LinkedIn version. ~750 words.

Here's a situation that probably sounds familiar. I've got two Claude Code sessions going in
two terminal tabs. One is grinding through a refactor — editing files, running tests, churning.
The other finished whatever I asked four minutes ago and has been sitting there waiting for me
to answer a question. I don't notice, because both tabs look exactly the same: a name, and some
text that stopped scrolling. So I sit there waiting on a tab that's waiting on me.

That's a small annoyance once. Run three or four agents at a time, all day, and it becomes a
real tax: you're constantly alt-tabbing through sessions to find the one that actually needs you.

The thing is, the terminal *could* tell me. It just doesn't.

## Terminals are blind on purpose

Modern terminal emulators are genuinely impressive — GPU-accelerated, beautiful font rendering,
fast as hell. But they render *glyphs*. They don't know anything about what's happening inside
them. To the terminal, an agent thinking hard for two minutes and an agent that stopped and is
waiting for your input look identical: a process attached to a pty that happens not to be
printing anything right now.

That was completely fine for fifty years, because *you* were the one typing every command. You
always knew the state of your own session, because you were driving it. The moment you start
delegating work to agents that run for minutes and then quietly wait, the terminal's blindness
becomes the bottleneck.

## What vibeflow does

vibeflow is a Linux terminal emulator with one core idea: every tab carries a small indicator
that tells you what the program inside it is *doing*.

> 📸 **[SCREENSHOT: the tab bar — one tab with a blue "working" stripe next to one with a
> pulsing amber "waiting" stripe. This is the money shot for this post and for LinkedIn.]**

A thin stripe runs down the left edge of each tab:

- **Blue** — the tool inside is working (generating, running commands).
- **Amber, gently pulsing** — it's waiting on you. This is the one that matters.
- **Gray** — idle at a prompt; a normal running command shows nothing special.

That's it. You glance at the tab bar and you know, at a distance, which conversation needs you
back — without alt-tabbing through all of them to check. The amber tab is the one to click.

## How it knows (the short version)

There are two ways vibeflow figures out a tab's state, and they stack.

The clean way is an open escape sequence — I call it **OSC 1338** — that a tool can emit to
announce its own state: "I'm working," "I'm waiting." Claude Code, for example, can be wired up
with a few hooks so it emits these as it goes. Any tool can adopt it; it's a published,
documented protocol, not a private feature.

The fallback, for tools that don't emit anything, is heuristic: vibeflow looks at which process
is in the foreground of each tab and watches its output. When a known AI tool goes quiet for a
few seconds after working, that's almost always "waiting on you," and the tab goes amber.

The first way is precise; the second works on day one with zero cooperation from the tool. If
you want the actual wire format, the detection timing, and the parts that were genuinely hard to
get right, I wrote a [deep-dive on how it works](LINK-TO-DEEP-DIVE-POST).

## The honest part

vibeflow is free and open source (MIT/Apache-2.0), GPU-rendered in Rust, and runs on Linux under
both X11 and Wayland. It's v0.1.x — early, useful daily for me, and rough in the places early
software is rough.

I should also be upfront: I built it as one person, I'm relatively new to Rust, and I leaned
heavily on AI agents (Claude Code) to write it. That felt fitting for a tool whose entire reason
to exist is making AI-assisted coding less annoying — but it also means I'd genuinely value more
eyes on it.

## Try it

```
cargo install vibeflow
```

…or grab the single-file AppImage from the
[releases page](https://github.com/bjhengen/vibeflow/releases/latest). Source and docs are on
[GitHub](https://github.com/bjhengen/vibeflow).

If you build terminal tools or AI coding tools, I'd especially love for you to look at the
OSC 1338 protocol. The whole point of making it an open escape sequence instead of a private
feature is that it gets more useful the more tools — and more terminals — speak it.
