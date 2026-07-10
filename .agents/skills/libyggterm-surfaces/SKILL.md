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
   signal panel, or a Cellulose ribbon lives. SHIPPED 2026-07-09 (ychrome's vault
   pane); `RightPanelMode::Vault` and `::AppSidebar` both DELETED 2026-07-10.
   Protocol below.
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
ESC ] 7717 ; <verb> ; <action> ; <base64-json> BEL
```

Two verbs ship today:

- `web-surface` — actions `open`, `heartbeat` (~4s, full payload), `close`.
  Payload `{"session": $YGGTERM_SESSION_ID, "url": "...", "title": "..."}`.
- `sidebar` — actions `declare` (idempotent, re-emitted on the heartbeat
  cadence), `close`. Payload
  `{"session", "control", "panes":[{id,icon,title}], "policy_version"?}`.

The GUI keys surface AND contribution state by the *stream* the OSC arrived on;
the `session` field is diagnostic (a remote session's env id lives in the remote
daemon's namespace and is not comparable to the GUI path). Authoritative yggterm-side doc: `docs/web-surfaces.md`. App-side
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

## Sidebar-contribution surface — SHIPPED 2026-07-09, live-proven

Built and live-proven on jojo (yggterm `010b9957`, ychrome `8452654`). ychrome's
vault pane is a CONTRIBUTION now, not yggterm chrome.

- **Declare**: the app emits `OSC 7717 ; sidebar ; declare ; <base64-json>`
  carrying only `{session, control, panes:[{id, icon, title}]}` — a loopback
  **control endpoint** URL plus the panes it offers. The schema is NOT in the
  OSC: the GUI `GET`s `<control>/pane/<id>` when a pane is opened, so a 1100-row
  vault never rides the PTY. Re-emitting `declare` on the heartbeat cadence is
  the liveness signal (idempotent — and it must NOT re-resolve the control URL,
  or you spawn one `ssh -L` per heartbeat); `sidebar ; close` retires it, and an
  unswept contribution expires like a surface. NO secrets in a schema, ever.
- **Render**: yggterm draws the schema with generic widgets in the right panel
  (`AppPaneRailBody`). It knows nothing about what the app means by them.
  Vocabulary: `section`, `label`, `search-box`, `text-input`, `number-input`,
  `toggle`, `button`, `list-row` (with action buttons), `tabs`. An unknown
  `kind` fails the pane rather than rendering a hole.
- **Act**: a click `POST`s `{pane, action, values}` to `<control>/action`; the
  app performs it on its own host and returns `{schema?, toast?, eval?}` — a
  fresh schema to re-render, a message to toast, and/or a script for the GUI to
  run in the surface. That is how a host-resident credential reaches a
  client-rendered page: the app computes, the GUI injects.
- **Page context**: the GUI passes the active surface's host as `?host=` on the
  schema GET and as `values.host` on an action. Non-secret context; the APP
  decides what a host means (which logins apply to it). One owner GUI-side:
  `ShellState::web_surface_host_label`.
- **Reaching the control endpoint**: a *loopback* URL on the app's host. The GUI
  fetches it over a plain `TcpStream`, so it needs an **`ssh -L` forward** — NOT
  the `ssh -D` SOCKS proxy the webview uses. Use `resolve_control_endpoint_url`;
  `resolve_web_surface_effective_url` returns early on the SOCKS branch, which is
  right for the webview and wrong for anything the GUI fetches itself.
- **Mode**: `RightPanelMode::AppPane(String)` carries the app's pane id, so the
  enum is not `Copy`. The rejected alternative — a unit variant plus a separate
  `active_app_pane: Option<String>` — is two encodings of one fact.
- **Richness escape hatch (v2)**: REJECTED for the vault pane (2026-07-09) — a
  full WebKitGTK webview in a 300px panel would not follow `DESIGN.md`, would
  render secrets inside a webview, and adds moving parts. Grow the vocabulary
  instead. Keep v2 in reserve for a pane that is genuinely a document.

### Trap: key contributed widgets by identity, never by index

Keying rendered widgets on their position let Dioxus patch a `section` node into
a `label` when a tab switch changed the widget at that slot — same tag, so the
node was reused and kept the section's `text-transform`. The Tools tab rendered
"UNLOCKED · 1107 ITEMS". Key on kind + id. Caught live, not by a test.

### Who owns a widget's value (settled 2026-07-10)

**The app owns every field's value.** A schema declares what each field holds;
yggterm's `app_pane_values` is only the user's edits *since that schema arrived*,
and applying a schema REPLACES it. Two consequences the implementation depends on:

- An app must **echo a draft back** in the schema it returns, or the field blanks.
  ychrome keeps the Add-tab draft in its own `PaneState` — host-resident, like
  everything else the app owns.
- A value the app stops declaring is **dropped**, which is what stops a typed
  password riding along on the next unrelated action's POST.

Inputs render with `initial_value` (uncontrolled), so a pushed value would be
silently ignored. Each widget id carries a **value epoch** that bumps only when an
applied schema declares a value the field is not already showing; the epoch rides
the Dioxus key, so a prefill rebuilds the node while an app echoing back what the
user typed leaves the caret alone.

### Secrets: the rule, precisely

The flow is **one-way**. A `secret` text-input carries what the user TYPED up to
the app on an action; the app declares it back **empty**. Never put a secret in a
schema, a declaration, or any OSC payload. An app that wants to hand the user a
generated secret does not echo it — ychrome's Add tab generates on save
(`--generate`), so the password is rolled on the app's host, encrypted, and stored
without ever entering yggterm. When a value must reach the *page*, use `eval`:
the app computes, the GUI injects, yggterm stores nothing.

### Driving a contributed pane headlessly

`server app right-panel pane:<id>` opens a pane the active app declared (e.g.
`pane:vault`) and fetches its schema — idempotent, unlike the titlebar button's
toggle. Before this existed the only way in was to click the button with
`app dom-eval`. yggterm does not know the pane ids; the app declares them.

### No app chrome remains (2026-07-10)

`RightPanelMode::Vault` and `RightPanelMode::AppSidebar` are both **DELETED**.
The contributed vault pane covers the password generator and the watchtower
(`vault_password_is_weak` moved to `ychrome-vault::watchtower`), and ychrome's
adblock + userscript settings are a second contributed pane (`pane:settings`).
`Metadata`, `Settings`, `Connect`, `Notifications`, `Hidden` and `AppPane(id)`
are all that is left, and every one of them is yggterm's own.

**If you are about to add a `RightPanelMode` variant, you are wrong.** Declare a
pane from the app.

### Shipping a policy the GUI must apply (adblock, userscripts)

Some app config cannot be applied host-side: ad blocking and userscripts act on
the GUI's webview. The app still OWNS them — it serves the *effective* policy and
the GUI applies it, the same shape as vault fill (app computes, GUI injects).

- `declare` carries `policy_version`: a **stat-only** stamp (paths + lengths +
  mtimes + the enabled/disabled decision), never the content. The GUI refetches
  `GET <control>/policy` only when the stamp moves, so a 10 KB ruleset does not
  ride the ~4s heartbeat.
- `/policy` answers `{adblock_rules: string|null, userscripts: [string]}` with
  every enable/disable decision already made. `null` means "no ad blocking",
  and the GUI never asks why.
- yggterm persists nothing but a content-addressed compiled-filter cache
  (`~/.yggterm/web-adblock-cache/<sha256>.json`), because WebKit's
  `UserContentFilterStore` compiles from a path rather than from memory.

**DECLARE BEFORE YOU OPEN.** A userscript only injects at document-start, so the
surface reconciler *holds* the lazy create while a declared policy is in flight
(`SurfacePolicyGate::Pending`). Emit `sidebar ; declare` before `web-surface ;
open` — including in the post-suspend re-emit — or the first apply pass sees a
surface with no contribution, creates it unblocked, and it runs without
userscripts forever. The gate opens after `MAX_POLICY_FETCH_ATTEMPTS` failures
(a page with no adblock beats no page) and the user is notified.

A surface opened by a **non-browser** app gets no adblock and no userscripts.
That is correct, not a gap: adblock is browsing config, and a dashboard is not
browsing.

### `reload_surface`, not `eval: "location.reload()"`

An action reply may set `reload_surface: true`. The GUI then drops the policy it
holds, refetches `/policy`, and **destroys and recreates** the webview.

Do not reach for `eval: "location.reload()"` here. A content filter and its
userscripts are bound to the WEBVIEW at creation, so reloading the *document*
leaves both attached — an app that just turned ad blocking off would watch the
toggle flip and nothing change. Only destroy-and-create applies a new policy, and
the policy must be refetched *before* the recreate or the surface comes back
under the rules the user just retired.

### ⚠ Trap: verifying ad blocking on a host with DNS-level ad blocking

`doubleclick.net`, `googlesyndication.com` and ~28 other names in jojo's ruleset
resolve to `::` / `0.0.0.0` — the network blackholes them. A `fetch()` to any of
them fails **whether or not the content filter is attached**, so probing them
proves nothing and reads as a pass. I "confirmed" adblock twice off that lie.

Check with `getent hosts <domain>` first. On jojo only two blocked domains
resolve for real: `c.amazon-adsystem.com` and `connect.facebook.net`. Also avoid
CSP-bearing pages (theguardian.com blocks the fetch by policy) — probe from
`example.com`. The honest test is an **A/B on one page**: toggle adblock, reload,
and require a neutral third-party (e.g. `cdn.jsdelivr.net`) to keep loading in
both states.

### Where an app's config lives when the app is remote (SHIPPED 2026-07-10)

The app's host owns the config, always — including ychrome's adblock rulesets
and userscripts. This was previously fudged ("only their application to the
GUI's webview stays host-side"), and it had no consistent meaning: the GUI's
webview read `~/.yggterm/web-adblock/*` **on the GUI host**, so an ychrome
running over ssh was editing files nothing read.

The rule: the app mutates its own host's config, and the control-endpoint
response **ships the effective ruleset/userscripts to the GUI**, which applies
them to the webview. Same shape as vault fill — the app computes, the GUI
injects, and yggterm persists none of it. A `RightPanelMode` for an app's
settings is still the anti-pattern; the settings pane is an app contribution
like any other. Mechanics: "Shipping a policy the GUI must apply", above.

ALT+/KeyTips is the keyboard surface — spec finalized 2026-07-10, see
[[campaign-alt-keytips-layer]] (reserved-letters namespace: shell KeyTips use
only non-Excel letters; apps claim Excel's F,H,N,P,M,A,R,W,X,Y,Q;
command-registry SSOT, contributions ride OSC 7717 ids).

## The launcher registry — SHIPPED 2026-07-10

A fifth surface, and the only one that does NOT ride OSC 7717: an app that is
merely *installed* must appear in the menus, whether or not it is running.

An app writes a manifest to **its own host** on every run:

```text
~/.yggterm/apps/<name>.json
{ "name": "ychrome", "label": "Ychrome", "icon": "",
  "binary": "/home/pi/.local/bin/ychrome",
  "verbs": [ { "id": "new", "label": "New Ychrome", "args": [] } ] }
```

- The host's **daemon** scans the directory, checks each `binary` still resolves,
  and **deletes the manifests of apps that are gone**. That is the entire
  uninstall story; the GUI keeps no registry of its own. It rides
  `ServerUiSnapshot::apps`, so menus are per-host by construction — an app on
  `dev` but not `jojo` appears on `dev` viewports only.
- `binary` must be **absolute**. A verb is launched by opening a terminal session
  and typing the command, and a fresh PTY has no login shell's `PATH` (the same
  trap that makes `ychrome` "not found" over `terminal send`).
- `name` must equal the file stem, or the manifest is ignored — one app cannot
  squat another's entry. A malformed manifest is ignored, never deleted: it may
  belong to a newer yggterm.
- Writing on **every run** is what repairs the recorded path after an upgrade.

GUI side: `app_launcher_entries(&snapshot.apps)` is the ONE derivation. The
titlebar `+` menu, the cwd-tree context menu and the start page all render it,
and `spawn_launch_app_verb` is the one launcher. Adding a surface must never mean
copying the list. The split-group compound-row menu joins them when it ships.

The hardcoded "New Paper" entries are **deleted** — Paper was never a libyggterm
app, just a stub the shell knew about. It comes back as a registry entry when a
Paper app ships one. Full design: [[project-libyggterm-app-menu-contribution]].

## Worked example: the password vault as an ychrome-owned surface

The native Bitwarden/Vaultwarden client (crate `ychrome-vault`, crypto proven
against a real 1107-item vault) was FIRST built inside the yggterm repo and
wired into a hardcoded yggterm sidebar. **That was the wrong ownership.** The
crate now lives in the ychrome repo, `rbw` was purged fleet-wide 2026-07-09,
and the hardcoded pane was deleted 2026-07-10. The migration is COMPLETE:

- **ychrome owns** the vault crate, the vault-agent (unlock cache, host-resident,
  auto-lock), the `ychrome-vault` CLI, the watchtower analysis, and the sidebar
  schema it declares. App-side contract: ychrome's own
  `.claude/skills/ychrome/SKILL.md`.
- **yggterm provides** only the generic sidebar-contribution surface that renders
  ychrome's declared schema and routes its actions — plus surface-eval for fill.
- **Host-resident**: the vault config and unlocked session live where ychrome
  runs (remote over ssh included); fill reaches the client-rendered page via
  surface-eval. This matches the egress rule: the host owns the network identity,
  so the host owns the browsing identity, so the host owns the vault.

Full vault execution plan (agent, writes/EncString-encrypt, passkeys, rbw
retirement): [[campaign-native-vault-client]] and ychrome's `docs/password-manager.md`.

## Anti-patterns (things this skill exists to prevent)

- App logic or an app icon in `yggterm-shell`. → contribution protocol.
- A daemon socket RPC for surface control. → OSC 7717 on the byte stream.
- yggterm persisting an app's data. → host-resident state.
- Designing the universal surface API before a second consumer needs it. →
  extraction-not-construction.
- A secret in a sidebar schema or an OSC payload. → the app performs the action;
  only non-secret metadata crosses the wire.
