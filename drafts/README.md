# Blog drafts — vibeflow launch writeup

Two-post package telling the "why an AI-coding-aware terminal should exist" story.

## Files

| File | What it is | Length | Primary venue |
|------|------------|--------|---------------|
| `01-overview-ai-aware-terminal.md` | Light front-door post: the pain, the feel, one screenshot. | ~750w | Personal blog → cross-post to Medium → condensed for LinkedIn. |
| `02-deep-dive-detecting-ai-state.md` | Technical deep-dive: OSC 1338 wire format, the 3 detection tiers, the `/proc` heuristic, honest limits. | ~1,800w | Personal blog. **This is the URL to submit to Hacker News.** |
| `03-linkedin-post.md` | ~150-word condensation of the overview + screenshot + link to the blog. | ~150w | LinkedIn. |

## Publishing plan

1. Publish both to the personal blog (deep-dive linked from the overview; fix the
   `LINK-TO-DEEP-DIVE-POST` placeholder in the overview once the URL exists).
2. Take it to Hacker News. On HN you submit a *link*, not text — discussion happens in
   the comments, and the author dropping one short context comment early is normal and
   welcome. Two clean ways to do it (don't mix them):
   - **Regular submission (recommended for the writeup):** submit the **deep-dive** blog
     URL with a plain descriptive title, e.g. "How vibeflow knows what your AI agent is
     doing". This is a "here's something I wrote" story; it lives or dies on the content.
   - **Show HN:** "Show HN" is specifically for something people can *try*, so it should
     link to the **repo** (or a landing page with install instructions), not the article.
     Title e.g. "Show HN: vibeflow – a Linux terminal that shows when your AI agent is
     waiting". Then link the deep-dive from the repo README and/or your first comment.
3. Cross-post the overview to Medium.
4. LinkedIn: post `03-linkedin-post.md` — attach the money-shot screenshot, fill in
   `LINK-TO-BLOG-POST`.

## TODO before publishing

- [ ] **Capture the money-shot screenshot** — tab bar with one tab blue/"working" next to
      one amber/"waiting". Used in the overview and on LinkedIn. (VNC session is available;
      can be captured live.) Search the files for `📸 [SCREENSHOT` to find every placeholder.
- [ ] Fill in URL placeholders once the blog posts are live: `LINK-TO-DEEP-DIVE-POST`
      (overview) and `LINK-TO-BLOG-POST` (LinkedIn). Grep: `grep -rn LINK-TO drafts/`.
- [ ] Sanity-check every technical claim in the deep-dive against current `main` before
      it hits HN (the crowd fact-checks). Source of truth: `docs/protocol.md`,
      `crates/vibeflow-protocol/src/lib.rs`, `crates/vibeflow/src/session/{tracker,proc_watch,osc}.rs`,
      `crates/vibeflow/src/render/tabs.rs`.

## Editorial decisions (so they don't get re-litigated)

- **Angle:** product/vision — "why AI coding needs this" — not a builder's-journey memoir.
- **Meta angle (AI-built, new-to-Rust):** present as a brief honest aside, not the thesis.
- **Prior art** (OSC 133 / iTerm2 / FinalTerm / tmux) is named up front in the deep-dive so
  HN doesn't have to; the novelty claim is "an *open* protocol for *agent state* + a `/proc`
  fallback," nothing broader.
