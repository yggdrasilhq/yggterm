# Spec — the agent-CLI installation modal

**One question this surface owns:** *which agent CLIs are on which machine, and
may yggterm fetch the ones that are missing?*

It is the second of the two Settings buttons that concern agent CLIs. The first
(`spec-agent-cli-extra-args-modal.md`) answers *how does a CLI launch*; this one
answers *is the CLI there at all*. They are deliberately two buttons: a user
tuning permission flags is doing different work from a user provisioning a new
machine, and the owner asked for them in that order — **install sits after
flags**, locked by
`the_cli_install_button_follows_the_launch_flags_button_in_the_settings_rail`.

## Why the surface exists

Measured 2026-08-20 across the fleet: one host carried all nine agent CLIs and
the host beside it carried **none of the eight** non-Claude ones — and no surface
anywhere reported the difference. Every symptom of it appeared somewhere else: a
spawn that could not be model-pinned "because the CLI is not installed locally",
a kind that refused a launch, a machine that silently could not host half the
product. The gap was never visible as itself.

## §1 The three states of consent, and why it is not a bool

`InstallConsent` (`yggterm-core/src/cli_install.rs`) has **three** values, and
the third is the one that matters:

| value | offer shown? | may fetch? |
|---|---|---|
| `Undecided` | yes | no |
| `Granted` | no | yes |
| `Declined` | no | no |

⛔ **`Declined` and `Undecided` must never be collapsed.** A bool cannot tell
"said no" from "never asked": collapsing them either nags a user who declined, or
installs for one who was never asked. Stored as the wire word in
`Settings::agent_cli_install_consent`, and read back ONLY through
`InstallConsent::from_wire`, which degrades anything unrecognised to `Undecided`
— a corrupt settings file must not be able to authorise fetching third-party
software.

## §2 Why the licence text lives in the modal

yggterm installs other people's programs: by package manager, and for at least
one CLI by running a vendor's install script. The acknowledgement therefore sits
**above the matrix that shows what would be installed and where**, not in a
checkbox elsewhere. Consent to an abstraction is not consent.

The wording states the separation plainly: each CLI is published by its own
vendor under its own licence, yggterm does not redistribute it, installing
fetches it into the user's own account, and nothing is fetched until they agree.

## §3 Presence: every machine probes ITSELF

`CliPresence` has four states and `Unknown` is a real one.

- **This machine** is probed by resolving each descriptor's `binary_name` on
  `PATH` (`probe_local_presence`). Deliberately a path lookup, not an execution:
  running `--version` per CLI per repaint costs a process each time, and at least
  one vendor CLI unpacks a payload on first invocation — a probe that expensive
  changes the machine it measures.
- **Every remote machine reports its OWN `PATH`** through
  `server remote cli-presence`, fetched on the existing machine-refresh ssh path
  beside the app-registry fetch. ⭐ **The remote runs the same core function on
  itself** that the GUI runs locally, so "is this binary here" has one
  implementation rather than an ssh-side reimplementation that could answer
  differently.
- ⛔ **Only the measured fact crosses the wire.** `CliPresenceReport` is
  `{slug, present, version}`. Display name, install method and whether a CLI is
  recommended are DERIVED from the registry by the receiver — a remote shipping
  its own idea of a display name would be a second registry that can disagree
  with the first.
- ⛔ **A slug the report omits stays `Unknown`, never `Absent`**, and a machine
  with no report at all renders *"not probed"*. That covers three different
  hosts — unreachable, never-refreshed, and running a yggterm older than the
  verb — none of which is a host that is missing its CLIs. A failed fetch KEEPS
  the machine's previous report rather than blanking it, the same
  `None`-vs-`Some(vec![])` discipline the app-registry fetch uses.

⛔ **`Unknown` must never render as `Absent`.** An unreachable host is not a host
missing its CLIs, and treating it as one makes the primary button offer installs
it cannot perform against machines it never contacted.

## §4 The recommendation is: everything, everywhere

`recommended_plans` returns every absent, unattended-installable CLI on every
machine. This is the owner's ruling of 2026-08-08 (`settled-calls.md`) expressed
as code rather than as prose. A row drops out of the plan only when it is already
present, not probed, unsupported on that platform, or not fetchable without a
human.

⛔ The function returns an **empty plan set when consent is not granted**, rather
than a plan a caller might run anyway. The gate travels with the plan so a caller
cannot hold one without the other.

⭐ A `Manual` CLI stays **recommended** — the owner wants it everywhere — but can
never enter an unattended plan, and is surfaced separately as *"install by hand"*.
Dropping it from the matrix would hide a CLI the user is expected to install;
putting it in the plan would promise an install that cannot happen.

## §5 What the button does today, and what it does not

The primary action runs a **foreground** managed-CLI refresh for **this machine**,
after re-reading consent at the point of fetch (the button is not the only path
to the code, so it is not the only gate).

⚠ **It does not yet sweep the fleet.** Remote **probing** now works (§3), so the
matrix tells the truth about every reachable machine; remote **installing** is
still the open half. The provisioner owns a fleet scope
(`ManagedCliRefreshScope::Fleet`, machine key `*`), and wiring the button to it
would promise the user work the modal cannot report progress for — per-machine
progress is what that half needs before the button can claim it.

⛔ **Do not "fix" that by relaxing the incidental install gate.**
`YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL` defaults off because that arm fires on
hot paths (focus, attach, launch) and an npm/uv run there was a measured CPU and
fan regression. Consent makes an install *permitted*; it does not make it free.
