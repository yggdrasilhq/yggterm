# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

## 2.0.25

### Fixed

- scope remote Codex session discovery to the actual SSH login user's `~/.codex`, so a machine no longer advertises sessions from a different account that the selected SSH target cannot resume
- preserve `remote-session://...` restore identity for cached live remotes even when the scanned session has disappeared, so launch prep still takes the remote resume path instead of silently downgrading to the wrong attach flow
- fall back from `codex resume <id>` to the interactive `codex resume` picker when a saved remote session ID is gone, keeping the terminal alive instead of closing the SSH tab on a stale restore

## 2.0.24

### Fixed

- keep only `yggterm.desktop` visible in Linux menus while leaving `dev.yggterm.Yggterm.desktop` as a hidden compatibility entry, so KDE no longer has two equally named menu candidates fighting over icon resolution
- force KDE desktop caches harder during integration by clearing stale per-user sycoca/icon caches, rebuilding with `--noincremental`, and nudging Plasma to refresh the current shell after the desktop files change

## 2.0.23

### Fixed

- point both Linux desktop entries at the shipped SVG file directly instead of relying on icon-theme name lookup, so KDE panel and menu paths can render the same icon artwork that already shows up correctly in desktop grid views

## 2.0.22

### Fixed

- disable the crashing GTK accessibility bridge by default on Linux unless `YGGTERM_ENABLE_ACCESSIBILITY=1` is set, which keeps jojo-style KDE/Wayland launches from dying in `libatk-bridge` before the window appears
- add a `yggterm.desktop` compatibility entry alongside `dev.yggterm.Yggterm.desktop`, plus extra SVG icon copies, so KDE menu and launcher lookup has both the strict desktop-id path and the plain legacy name available
- make direct-install launchers and packaged wrappers carry the same Linux accessibility guard before GTK/WebKit boot, so launcher/menu starts behave like direct binary starts instead of crashing differently
- ship `yggterm-mock-cli` in release archives, direct installs, and the `.deb`, so native startup, daemon, and integration issues can be diagnosed with the same installed tool users have on their machines

## 2.0.21

### Fixed

- corrected the shipped PNG icon so the runtime window icon and KDE launcher icon finally match the canonical SVG artwork instead of showing the broken raster fallback
- aligned Linux desktop integration around the `dev.yggterm.Yggterm` desktop identity, including duplicate icon names for the desktop file and theme cache refreshes that KDE can actually resolve
- replaced the old hide-on-close desktop behavior with a real client shutdown path, so closing one live client no longer leaves a hidden stale process behind
- kept restart-into-update from tearing down the daemon in between client handoff, so a client restart does not unnecessarily kill running sessions
- added a lightweight `--version` / `-V` CLI path so launcher and diagnostic checks do not accidentally boot the full GUI

## 2.0.20

### Fixed

- replaced the direct-install Linux launcher with a stable wrapper that reads `install-state.json`, so stale symlinks can no longer leave the desktop entry or shell command pinned to an older broken binary after self-update
- made direct self-update hand desktop integration refresh to the freshly installed binary, instead of trusting the older running binary to rewrite launchers and icons correctly
- aligned the shipped PNG window/launcher icon asset with the canonical SVG source so the runtime icon and installed desktop icon stop drifting
- stopped remote scan helper commands from panicking on broken stdout pipes during startup and shutdown races

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
