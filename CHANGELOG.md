# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

## 2.0.19

### Fixed

- normalize remote machine aliases during restore and connection so alternate host aliases map to canonical machine identities consistently, improving remote session continuity across reconnects.

## 2.0.18

### Fixed

- make Linux direct installs register the desktop launcher like a direct app instead of a distro package: stable `Exec` via `~/.local/bin/yggterm` and a stable absolute `Icon` path under `~/.local/share/yggterm/direct/`
- keep the theme/pixmaps icon copies as fallback, but stop relying on KDE theme-name resolution alone for the primary launcher icon
- publish release checksum sidecars consistently so direct self-update does not fail on missing `.sha256` assets
- only shut down Yggterm server sessions when the last live client closes, so closing one of multiple open windows no longer tears down the others
- treat Linux termination signals like a graceful close path too, so KDE panel/taskbar close does not bypass daemon shutdown semantics

## 2.0.17

### Fixed

- make remote Codex discovery resilient when an SSH target logs in as a different user than the one that owns the session archive, so default `~/.codex` scans still find machine-wide homes like `/home/pi/.codex`
- keep `yggterm-headless` robust for root-login remotes by letting the remote scan path fall back to real machine user homes instead of reporting a misleading healthy `0 sessions`

## 2.0.16

### Fixed

- make stale `offline` SSH machines refresh again on startup instead of being treated as already-known forever
- add a cooldown to automatic remote machine refresh retries so a bad host does not spin forever
- refresh Linux desktop integration more aggressively for KDE by installing the themed icon into `pixmaps/` and forcing both icon and desktop menu cache updates
- keep direct self-update installing `yggterm-headless` alongside `yggterm`

### Docs

- added a standalone product thesis in `PRODUCT_THESIS.md`
- rewrote the README opening to better explain the core user, pain, and wedge
