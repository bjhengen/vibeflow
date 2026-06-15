# Security Policy

## Supported versions

vibeflow is pre-1.0; only the latest released version receives security fixes.

| Version | Supported |
| ------- | --------- |
| latest release (see [Releases](https://github.com/bjhengen/vibeflow/releases)) | yes |
| older releases | no |

## Reporting a vulnerability

Please report security issues **privately** — a terminal emulator parses
attacker-controlled byte streams, so escape-sequence and clipboard bugs can
have real impact for users.

- Email: **bhengen@gmail.com** (subject line starting with `[vibeflow security]`)
- Or use GitHub's private vulnerability reporting on this repository
  (Security tab → "Report a vulnerability"), if available.

Please include a proof-of-concept byte sequence or reproduction steps where
possible. You can expect an acknowledgement within a few days. Once a fix is
released, the issue will be disclosed in the changelog with credit (unless you
prefer otherwise).

Please do **not** open public issues for unpatched vulnerabilities.

## Scope notes

- vibeflow intentionally does **not** implement OSC 52 clipboard *read*
  (clipboard exfiltration vector); reports that it does would be a bug.
- OSC 52 clipboard *write* is honored by default (so `vim "+y`, tmux
  pass-through, and remote-SSH copy work), which means terminal output can set
  the system clipboard. If you don't want untrusted output to be able to do
  that, disable it:

  ```toml
  [clipboard]
  allow_osc52_write = false
  ```

- Outside bracketed-paste mode, pasted clipboard content (including embedded
  newlines) is forwarded to the child verbatim — matching the behavior of
  mainstream terminals. The bracketed-paste *end* marker is stripped to defeat
  the "paste prematurely ends the bracketed frame" injection, and every modern
  shell/editor enables bracketed paste, which neutralizes newline auto-execution.
- The OSC 1338 protocol parser (`vibeflow-protocol`) is fuzzed; crashes found
  by fuzzing it are in scope and very welcome.
