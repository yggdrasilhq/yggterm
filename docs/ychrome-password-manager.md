# ychrome × Bitwarden/Vaultwarden: autofill + passkeys design

Status: SLICE 1 SHIPPED (2026-07-09) — autofill MVP; TOTP/passkeys still design.
Companion to the ychrome daily-browser campaign. User setup: Bitwarden clients
against a self-hosted Vaultwarden, passkeys stored in the vault.

## Shipped: autofill MVP (slice 1, 2.9.66)

- `yggterm server app web fill [--session <path>]` + the app-sidebar
  "Fill login from vault" button (▦ pane, visible while a surface is live).
- Flow: the GUI reads the surface's REAL page URI from the engine
  (`web_surface_page_state` — the page cannot lie about it), refuses non-https
  non-loopback pages, and queries `rbw` for an entry whose NAME matches the
  host exactly (or its `www.`-stripped twin; rbw's list has no URI field, so
  entry naming is the matching contract). The credential is injected via the
  engine eval path — prototype value setters + input/change events so
  React-class forms see it — with an in-page toast naming the filled entry.
  Key material never rides an HTTP bridge; there is no loopback server in this
  slice (that design below remains for the passkey shim, which page JS must
  originate).
- Requires on the GUI host: `rbw` installed + configured
  (`rbw config set base_url <vaultwarden>`, `rbw register`, `rbw login`) and
  unlocked (`rbw unlock`). Errors surface as a GUI notification / CLI reason:
  no rbw, vault locked, no entry named for host, no visible password field.
- Deliberately NOT in slice 1: multi-match picker (first sorted match wins,
  chosen entry+username are reported in the response), TOTP, iframe fill,
  in-page autofill affordance (userscript detect+prompt).

## Engine reality (constrains everything)

- ychrome surfaces are WebKitGTK. **WebKitGTK has no WebAuthn implementation**
  (tracked upstream; GNOME Web has the same gap), so `navigator.credentials`
  does not exist natively and no browser-extension route exists either.
- Chrome's Bitwarden extension does passkeys WITHOUT platform FIDO2 APIs: it
  **overrides `navigator.credentials.create/get` in page JS** and answers the
  ceremony from vault-stored FIDO2 credentials. That is exactly reproducible
  with our userscript substrate (document-start, top frame, per profile).
- The Linux ecosystem fix (credentialsd / XDG credentials portal, FOSDEM 2026)
  is arriving but WebKitGTK would still need to consume it — not actionable now.

## Architecture: userscript shim + local signer bridge

```
page ──navigator.credentials shim (userscript)──▶ fetch http://127.0.0.1:<port>/fido2/...
                                                   │  yggterm GUI-host bridge (loopback,
                                                   │  token-authed, per-origin prompts)
                                                   ▼
                                     local signer: goldwarden (preferred) or rbw/bw
                                                   ▼
                                             Vaultwarden
```

- **Signer**: [goldwarden](https://github.com/quexten/goldwarden) is a
  Bitwarden-compatible Linux daemon with FIDO2/WebAuthn signing, biometric
  gating, SSH-agent — and works against Vaultwarden. Preferred backend: we
  bridge, it owns key material + user approval. Fallback for plain
  passwords/TOTP: `rbw` (agent-cached CLI).
- **Bridge**: a loopback HTTP endpoint owned by the yggterm GUI host (NOT per
  ychrome process): `/fill?origin=` (password/TOTP lookup), `/fido2/create`,
  `/fido2/get`. Bearer token injected into the userscript at surface build time
  so arbitrary local processes can't query the vault. Every request is
  origin-checked against the surface's actual page origin (the bridge knows it
  from the reconciler; the page cannot lie about it).
- **Userscript**: polyfills `navigator.credentials` when missing; adds an
  autofill affordance (detect login forms, fill from `/fill`, small picker on
  multiple matches). Same file works in every profile; per-profile disable by
  the settings pane.

## Slices

1. **Autofill MVP**: rbw-backed `/fill` + form-fill userscript (no passkeys yet).
   Origin-exact matching only; no iframe fill (top frame injection already
   enforces this); explicit per-fill toast.
2. **TOTP**: same endpoint returns the rolling code, fill on demand.
3. **Passkeys**: goldwarden bridge for `create`/`get` ceremonies; WebAuthn shim
   emulating CTAP2-over-vault exactly like the Chrome extension. RP ID
   validation in the bridge (origin ↔ rpId suffix rules), user-presence prompt
   via yggterm toast/dialog before every assertion.
4. **Cross-machine**: nothing to sync — Vaultwarden is the state; each machine
   runs its own signer. Matches the HOST-resident-state doctrine.

## Security invariants

- Key material never enters page JS; the shim only ferries CBOR/JSON blobs.
- Bridge binds 127.0.0.1, per-session bearer token, origin allow-check server-side.
- User-presence confirmation (dialog) before create/get — the agent may drive
  everything EXCEPT that consent by policy.
- Temp profile: autofill available, but nothing persisted page-side.

Sources: WebKitGTK WebAuthn gap — gitlab.gnome.org/GNOME/epiphany work item 1007,
fosdem.org/2026 credentialsd talk; goldwarden — github.com/quexten/goldwarden;
Bitwarden vault passkeys — bitwarden.com/help/storing-passkeys.
