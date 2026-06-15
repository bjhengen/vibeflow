# LinkedIn post — condensed from the overview

> **Draft — LinkedIn.** ~150 words, condensed from `01-overview-ai-aware-terminal.md`.
> Attach the money-shot screenshot to the post; link to the full blog write-up.
> Dropped "fast as hell" for the professional register (per the tone review).

> 📸 **[ATTACH: tab-bar screenshot — a blue "working" tab next to a pulsing amber
> "waiting" tab. This is the visual that earns the click.]**

---

I kept losing track of which AI coding session needed me — so I built a terminal that shows it at a glance.

Run two or three AI agents at once and they all look identical in the terminal: a tab that stopped scrolling. You can't tell "thinking hard" from "waiting on your answer," so you alt-tab around hunting for the one that's stuck on you.

vibeflow puts a small stripe on every tab — blue when the tool is working, pulsing amber when it's waiting on you. Glance at the tab bar; click the amber one.

Two ways it knows: tools can emit an open escape sequence (OSC 1338) to report their own state, and for everything else vibeflow infers it from process activity and output silence.

Free and open source, GPU-rendered in Rust, Linux, early days (v0.1). Built solo and largely with AI agents — fitting for a tool aimed at making AI-assisted coding less annoying.

Full write-up + how it works 👇
LINK-TO-BLOG-POST

#Rust #AI #DeveloperTools #OpenSource #Terminal
