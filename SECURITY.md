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
- The OSC 1338 protocol parser (`vibeflow-protocol`) is fuzzed; crashes found
  by fuzzing it are in scope and very welcome.
