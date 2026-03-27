# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

## 2.0.14

### Fixed

- ship `yggterm-headless` in release archives, direct installs, and `.deb` packages so SSH remotes receive the headless server binary instead of the GUI app
- make remote command transport resilient to noisy shell startup output by stripping protocol payloads after a Yggterm-owned sentinel marker
- recover from stale cached remote-binary paths by clearing the cache and retrying remote command resolution once
- rename `mock-yggclient` to `yggterm-mock-cli`

### Docs

- added a standalone product thesis in `PRODUCT_THESIS.md`
- rewrote the README opening to better explain the core user, pain, and wedge
