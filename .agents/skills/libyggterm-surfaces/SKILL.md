---
name: libyggterm-surfaces
description: How a program running in a yggterm terminal takes over the GUI's surfaces (viewport, cwd-tree document, sidebar panel, chooser) — the libyggterm app-platform contract. Read this before building ANY libyggterm app (quick-and-dirty or heavyweight), before touching web-surface / RightPanel / OSC 7717 code, or before adding app-specific chrome to yggterm.
---

# libyggterm surfaces

**libyggterm is an ncurses-analog for GUI chrome.** A program run in a yggterm
terminal — local or over ssh — can take over parts of yggterm's GUI: the
viewport becomes the app's web/dashboard view, a right-hand **sidebar panel**
carries the app's icons and controls, its documents appear in the cwd tree, and
a chooser can gate launch. The app is written like a CLI program; it inherits
yggterm's agent-first tooling (screenshots, traces, deterministic automation)
for free.

**ychrome is the pilot app** (a web browser). yedit, ytop, Paper, Cellulose are
the next consumers. This skill is the contract they all share.

## The one rule that governs everything

**yggterm provides the surface INTERFACE. The app OWNS the surface content.**

yggterm must contain **zero app-specific chrome**. There is no "vault sidebar"
in yggterm, no "ytop metadata panel," no hardcoded app menu entry. yggterm
offers a generic mechanism for an app to contribute a surface; the app supplies
what goes in it, owns the logic behind it, and owns any crate that implements
that logic. If you find yourself adding `RightPanelMode::Vault`, an app's icon,
or an app's business logic to `yggterm-shell`, stop — that belongs in the app's
own repo, wired in through the contribution protocol below.

Corollary — **host-resident state** (see [[project-libyggterm-platform-vision]]):
a libyggterm app is host-resident. Its state — documents, profiles, credentials,
the unlocked-vault session — lives on the host the app RUNS on (the invoking
host, which may be remote over ssh). yggterm is a pure renderer/controller and
persists **none** of the app's data. ychrome already follows this: its profile
jars live at `~/.yggterm/web-profiles/<name>/` on the invoking host.

Corollary — **extraction, not construction**: don't build a generic surface API
in the abstract. Build the minimum an app actually needs now; extract the shared
abstraction when a *second* app needs the same thing. A rich sidebar for one app
is a feature of that app until a second app wants sidebars.

## The four surfaces (taxonomy)

Build each only when an app truly needs it.

1. **Viewport surface** — the main pane becomes the app's view. SHIPPED for
   ychrome (a WebKitGTK child webview). Includes a viewport-mode toggle
   (Web ↔ Terminal), which should generalize so any app registers modes.
2. **cwd-tree document surface** — an app document appears as a node in the
   host's cwd tree, with open/export/share affordances. First real need:
   Cellulose (a sqlite spreadsheet shareable as .xlsx). NOT built.
3. **Sidebar-contribution surface** — the app contributes a right-hand panel of
   icons/controls/metadata. This is where a password-manager sidebar, an ytop
   signal panel, or a Cellulose ribbon lives. Protocol below; currently only a
   hardcoded MVP exists and must be generalized.
4. **Chooser / identity surface** — a picker before launch (profile, workspace,
   vault account). SHIPPED for ychrome's no-arg profile picker.

## Transport: OSC 7717 on the terminal byte stream

The control channel is **not** a daemon socket RPC (an early draft proposed one;
it was rejected). The app writes OSC escape sequences to its own stdout; the PTY
relay already carries them from any machine to the GUI (remote daemon → ssh
bridge → local daemon → xterm.js), so there is no discovery, no version
negotiation, no new transport, and unknown OSCs are invisible in a plain
terminal — that is the degradation story.

```
ESC ] 7717 ; web-surface ; <action> ; <base64-json> BEL
```

Shipped actions: `open`, `heartbeat` (~4s, full payload), `close`. Payload:
`{"session": $YGGTERM_SESSION_ID, "url": "...", "title": "..."}`. The GUI keys
surface state by the *stream* the OSC arrived on; the `session` field is
diagnostic. Authoritative yggterm-side doc: `docs/web-surfaces.md`. App-side
view: ychrome `docs/protocol.md`.

**Detecting thin-client mode:** the daemon exports `YGGTERM_SESSION_ID` and
`YGGTERM_BIN` into every PTY. Present ⇒ you are inside yggterm, use surfaces.
Absent ⇒ run as a standalone window (ychrome opens a tao/wry window).

**Lifecycle the app must honor:** emit `open` once; `heartbeat` every ~4s while
alive (the GUI expires a surface after 15s of silence, so a SIGKILLed app never
leaks an overlay); emit `close` on SIGINT; block in the foreground while the
surface is open (a surface is a foreground program, not a session). Heartbeats
must NOT be able to create or navigate a surface — only an explicit `open` can
(learned the hard way: heartbeats clobbering user navigation, see
[[finding-ychrome-usability-2966]]).

**When an action must reach back into the page** (e.g. fill a credential into
the surface's webview), the app cannot touch the GUI-owned webview directly. It
asks the GUI to run JS in the surface via the surface-eval capability (the
mechanism behind `server app web eval` / `app web fill`). The app computes the
value; the GUI injects it. This is how a host-resident credential reaches a
client-rendered page without the secret living in yggterm.

## Building a libyggterm app — quick-and-dirty to heavyweight

Minimum viable app (what ychrome does):

1. Parse args with clap. If `YGGTERM_SESSION_ID` is set, run thin-client;
   otherwise open a standalone window.
2. Emit `open` with your URL/title, then loop emitting `heartbeat` every ~4s,
   blocking in the foreground.
3. Handle SIGINT → emit `close` → exit.
4. Keep all your state on the invoking host under `~/.yggterm/<your-app>/`.

That is a whole viewport-surface app. A heavyweight app adds a sidebar
contribution (below), cwd-tree documents, and a chooser — each via the same OSC
channel plus a loopback control endpoint for actions.

## Sidebar-contribution surface — the protocol to build

This is **Open question 1** in ychrome's protocol doc and the current top of the
libyggterm work. The MVP that exists today (the app-sidebar `▦` pane and a vault
`🔑` pane) is **hardcoded in `yggterm-shell` and is the wrong shape** — it must
become a generic contribution the app declares.

**What must migrate out of yggterm (the hardcoded chrome to delete):** the
`RightPanelMode` enum in `yggterm-shell` today has two ychrome-specific variants
that are the anti-pattern — `Vault` (the Bitwarden pane) and `AppSidebar`
(ychrome's settings: adblock + userscript toggles). BOTH must become ychrome
contributions. The adblock rulesets and userscript files are already
host-resident ychrome-owned config; only their application to the GUI's webview
stays host-side (like vault fill). The other variants — `Metadata`, `Settings`
(yggterm's own), `Connect`, `Notifications` — are yggterm's own chrome and stay.

Target protocol (design; not yet built):

- **Declare**: the app emits an OSC 7717 verb (`sidebar`) whose payload is a
  declarative schema — a title, an icon, and sections of rows/controls from a
  fixed widget vocabulary (label, text-input, button, toggle, list-row,
  search-box), plus a loopback **control endpoint** URL (the picker already
  proved GUI-GETs-a-loopback-URL). NO secrets in the schema, ever.
- **Render**: yggterm draws the schema with generic widgets in the right panel.
  It knows nothing about what the app means by them.
- **Act**: a click sends the row/action id to the app's control endpoint; the
  app performs the action (unlock, fill, generate…) on the invoking host and
  returns updated state or a fresh schema. Page-touching actions use
  surface-eval as above.
- **Live update**: the heartbeat may carry a new schema (or the app pushes one);
  the panel re-renders.
- **Richness escape hatch (v2)**: when the fixed vocabulary is too poor (a full
  vault UI with search + generator + watchtower), the app instead serves its
  sidebar as an HTML page and yggterm docks a WebKitGTK child webview in the
  right-panel region — the same hosting it already does for the viewport, new
  geometry only. The app (being a browser-class program) owns the HTML and
  bridges it to its own logic. Prefer v1's declarative schema first
  (extraction-not-construction); reach for v2 only when a real app's panel can't
  be expressed declaratively.

Menu contributions (the titlebar `+` menu) are the same idea for a different
surface: an app-registry the shell reads instead of hardcoded arms — see
[[project-libyggterm-app-menu-contribution]]. ALT+/KeyTips is the keyboard
surface — see [[project-alt-keytips-layer]].

## Worked example: the password vault as an ychrome-owned surface

The native Bitwarden/Vaultwarden client (crate `yggterm-vault`, crypto proven
against a real 1107-item vault) was FIRST built inside the yggterm repo and
wired into a hardcoded yggterm sidebar. **That was the wrong ownership.** The
correct design, being migrated:

- **ychrome owns** the vault crate, the vault-agent (unlock cache, host-resident,
  auto-lock), the `ychrome vault …` CLI, and the sidebar schema it declares. The
  crate moves into the ychrome repo.
- **yggterm provides** only the generic sidebar-contribution surface that renders
  ychrome's declared schema and routes its actions — plus surface-eval for fill.
- **Host-resident**: the vault config and unlocked session live where ychrome
  runs (remote over ssh included); fill reaches the client-rendered page via
  surface-eval. This matches the egress rule: the host owns the network identity,
  so the host owns the browsing identity, so the host owns the vault.

Full vault execution plan (agent, writes/EncString-encrypt, passkeys, rbw
retirement): [[campaign-native-vault-client]] and `docs/ychrome-password-manager.md`.

## Anti-patterns (things this skill exists to prevent)

- App logic or an app icon in `yggterm-shell`. → contribution protocol.
- A daemon socket RPC for surface control. → OSC 7717 on the byte stream.
- yggterm persisting an app's data. → host-resident state.
- Designing the universal surface API before a second consumer needs it. →
  extraction-not-construction.
- A secret in a sidebar schema or an OSC payload. → the app performs the action;
  only non-secret metadata crosses the wire.
