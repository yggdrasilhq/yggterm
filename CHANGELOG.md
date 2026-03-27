# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

## 2.0.15

### Fixed

- make direct self-update install `yggterm-headless` alongside `yggterm` so live direct installs can actually repair SSH remote health on upgrade
- register the Linux desktop file with `Icon=yggterm` so KDE resolves the shipped icon consistently across the menu, panel, and launcher editor
- keep the remote command transport resilient to noisy shell startup output by stripping protocol payloads after a Yggterm-owned sentinel marker
- keep the helper binary renamed as `yggterm-mock-cli`

### Docs

- added a standalone product thesis in `PRODUCT_THESIS.md`
- rewrote the README opening to better explain the core user, pain, and wedge
