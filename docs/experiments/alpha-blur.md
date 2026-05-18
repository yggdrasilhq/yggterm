# Alpha Blur Experiment

Branch: `experimental/alpha-blur`

Worktree: `~/gh/yggterm--alpha-blur`

Channel binary: `yggterm-alpha-blur`

## Goal

Explore alpha, blur, and compositor-sensitive shell styling without changing
stable theme behavior on `main`.

## Guardrails

- Stable theme code must not depend on alpha, blur, grain, or compositor timing.
- The experiment must keep blur and transparency explicit, observable, and easy
  to disable.
- The channel must not overwrite the stable `yggterm` launcher, app id, update
  channel, or direct-install metadata.
- Use an isolated `YGGTERM_HOME` unless explicitly testing stable-home
  migration after a snapshot.

## Promotion Criteria

- Linux/KDE, macOS, and Windows behavior is understood.
- App-control exposes enough theme state to prove stable and experimental paths
  are distinct.
- Screenshots and smokes show readable chrome and terminal content in supported
  themes.
- Stable `docs/theme.md` is updated before any behavior lands on `main`.
