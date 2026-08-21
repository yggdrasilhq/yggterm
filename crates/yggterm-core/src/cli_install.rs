//! Which agent CLIs belong on which machine, whether they are there, and
//! whether the user has agreed to yggterm fetching them.
//!
//! ⛔ **ONE owner for three questions that were answered in three places.**
//! The provisioner asked "may I fetch this?", the launcher asked "is it here?",
//! and nothing at all asked "which machines should have it?" — so the answer to
//! the third was whatever a machine happened to accumulate. Measured 2026-08-20:
//! the fleet's GUI host carried NONE of the eight non-Claude CLIs, while the
//! host beside it carried all nine, and no surface reported the difference.
//!
//! # Why consent is a TYPE here and not a checkbox somewhere
//!
//! yggterm fetches third-party CLIs — some by `npm`, one by piping a vendor's
//! `install.sh`. Those are other people's programs under other people's
//! licences, and installing them is a thing the USER does, with yggterm as the
//! mechanism. Keeping [`InstallConsent`] in the core, beside the plan it gates,
//! means the licence question travels with the action it authorises instead of
//! living in a settings blob that a later caller can forget to read.
//!
//! ⚖ The BEHAVIOUR this gates is owner-settled (`docs/settled-calls.md`,
//! 2026-08-08): *auto install and update ALL CLIs on ALL connected systems
//! including localhost.* Consent does not re-litigate that ruling — it is the
//! licence acknowledgement the ruling always implied and never had a place for.
//! Once granted, the recommended plan is the ruling: everything, everywhere.

use crate::agent_cli::{AgentCliDescriptor, CliInstall, AGENT_CLIS};
use serde::{Deserialize, Serialize};

/// Whether the user has acknowledged that yggterm may fetch third-party CLIs
/// on their behalf.
///
/// ⛔ `Undecided` is NOT `Declined`. A machine that has never been asked must
/// not be treated as a refusal — the modal exists to turn the first into one of
/// the other two, and a caller that collapses them will either nag a user who
/// said no or install for one who was never asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InstallConsent {
    /// Never asked. Show the offer; install nothing.
    #[default]
    Undecided,
    /// The user accepted that yggterm may fetch these CLIs for them.
    Granted,
    /// The user declined. Diagnose and report, but never fetch.
    Declined,
}

impl InstallConsent {
    /// May yggterm FETCH a CLI right now? The one predicate the provisioner
    /// gates on, so "did they agree" is never re-derived from a settings string.
    pub fn may_fetch(self) -> bool {
        matches!(self, Self::Granted)
    }

    /// Should the offer be shown? True only while the question is genuinely
    /// open — a decline is an answer and must not re-prompt.
    pub fn should_offer(self) -> bool {
        matches!(self, Self::Undecided)
    }

    /// The stored wire word. Round-trips through [`Self::from_wire`].
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Undecided => "undecided",
            Self::Granted => "granted",
            Self::Declined => "declined",
        }
    }

    /// Read a stored value. ⚠ An unreadable or absent value degrades to
    /// `Undecided`, never to `Granted`: a corrupt settings file must not be
    /// able to authorise fetching someone else's software.
    pub fn from_wire(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "granted" => Self::Granted,
            "declined" => Self::Declined,
            _ => Self::Undecided,
        }
    }
}

/// Whether a CLI's binary is on a given machine.
///
/// ⚠ `Unknown` is a real state and must be rendered as one. A machine that is
/// offline, or that has not been probed yet, is not a machine that is missing
/// its CLIs — reporting it as `Absent` invents work and, worse, makes the
/// "install everything" action look like it has something to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CliPresence {
    Present { version: Option<String> },
    Absent,
    /// The CLI does not run on this machine's platform at all.
    UnsupportedHere,
    /// Not probed, or the probe could not reach the machine.
    Unknown,
}

impl CliPresence {
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present { .. })
    }

    /// Is this a row an install could actually change? Only a known-absent CLI
    /// qualifies — see the `Unknown` warning on the type.
    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

/// How a missing CLI would arrive, in words a human can act on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArrivalPlan {
    /// yggterm can fetch it with no human in the loop, once consent is granted.
    Unattended,
    /// yggterm cannot fetch this one; the human must install it themselves.
    NeedsHuman,
}

impl ArrivalPlan {
    pub fn for_install(install: CliInstall) -> Self {
        if install.provisions_unattended() {
            Self::Unattended
        } else {
            Self::NeedsHuman
        }
    }
}

/// One CLI's standing on one machine — the row the modal draws.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CliMachineRow {
    pub slug: &'static str,
    pub display_name: &'static str,
    pub binary_name: &'static str,
    pub presence: CliPresence,
    pub arrival: ArrivalPlan,
    /// Whether yggterm RECOMMENDS this CLI on this machine. The default answer
    /// is yes for every CLI on every machine (owner ruling); it turns false
    /// only where the CLI cannot run there at all.
    pub recommended: bool,
}

impl CliMachineRow {
    /// Would "install everything recommended" act on this row?
    ///
    /// ⛔ Three conditions, and dropping any one of them produces a plan that
    /// lies: it must be recommended, it must be genuinely absent (not merely
    /// unprobed), and yggterm must be able to fetch it without a human.
    pub fn is_installable(&self) -> bool {
        self.recommended
            && self.presence.is_actionable()
            && matches!(self.arrival, ArrivalPlan::Unattended)
    }

    /// A row the human must act on themselves: wanted, missing, unfetchable.
    pub fn needs_human(&self) -> bool {
        self.recommended
            && self.presence.is_actionable()
            && matches!(self.arrival, ArrivalPlan::NeedsHuman)
    }
}

/// Every CLI's standing on one machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MachineCliStatus {
    /// The machine key the rest of yggterm addresses this host by. Empty string
    /// is reserved for the local machine, matching `--machine-key`'s own
    /// convention, so a caller never has to special-case "here".
    pub machine_key: String,
    pub display_label: String,
    pub rows: Vec<CliMachineRow>,
}

impl MachineCliStatus {
    /// Build the full per-CLI standing for a machine from a presence lookup.
    ///
    /// The registry is walked in its own order so every machine's column reads
    /// the same way — a matrix whose rows reorder per host cannot be scanned.
    pub fn build(
        machine_key: impl Into<String>,
        display_label: impl Into<String>,
        mut presence_of: impl FnMut(&AgentCliDescriptor) -> CliPresence,
    ) -> Self {
        let rows = AGENT_CLIS
            .iter()
            .filter(|descriptor| descriptor.slug != "shell")
            .map(|descriptor| {
                let presence = presence_of(descriptor);
                CliMachineRow {
                    slug: descriptor.slug,
                    display_name: descriptor.display_name,
                    binary_name: descriptor.binary_name,
                    recommended: !matches!(presence, CliPresence::UnsupportedHere),
                    arrival: ArrivalPlan::for_install(descriptor.install),
                    presence,
                }
            })
            .collect();
        Self {
            machine_key: machine_key.into(),
            display_label: display_label.into(),
            rows,
        }
    }

    pub fn present_count(&self) -> usize {
        self.rows.iter().filter(|row| row.presence.is_present()).count()
    }

    pub fn installable(&self) -> impl Iterator<Item = &CliMachineRow> {
        self.rows.iter().filter(|row| row.is_installable())
    }

    pub fn needing_human(&self) -> impl Iterator<Item = &CliMachineRow> {
        self.rows.iter().filter(|row| row.needs_human())
    }

    /// A one-line summary for the machine's header row.
    pub fn summary(&self) -> String {
        let total = self.rows.len();
        let present = self.present_count();
        let missing = self.installable().count();
        let manual = self.needing_human().count();
        let unknown = self
            .rows
            .iter()
            .filter(|row| matches!(row.presence, CliPresence::Unknown))
            .count();
        if unknown == total && total > 0 {
            return "not probed".to_string();
        }
        let mut parts = vec![format!("{present}/{total} installed")];
        if missing > 0 {
            parts.push(format!("{missing} can be installed"));
        }
        if manual > 0 {
            parts.push(format!("{manual} need you"));
        }
        if unknown > 0 {
            parts.push(format!("{unknown} unknown"));
        }
        parts.join(" · ")
    }
}

/// Is this CLI's binary resolvable the way a LAUNCH on this machine will
/// resolve it?
///
/// ⛔ **The resolver is injected because the answer is not this crate's to
/// give.** A `PATH` lookup in the calling process answers "what can THIS
/// process exec", and that is a different question from "what will a session
/// yggterm starts here exec" — the launch prepends the managed CLI bin dir and
/// the login shell's dirs, neither of which a daemon or a GUI necessarily
/// carries. Measured on one fleet machine, the two answers were 1/10 and 10/10
/// at the same instant. The owner of launch resolution lives in the server
/// crate beside the launch itself; core takes it as an argument so there can
/// never be a second, quieter copy here.
///
/// ⛔ **This answers for the machine the caller is running on, and nothing
/// else.** The GUI host and the hosts it shows rows for are different machines
/// with different resolution — the fault this whole module exists to surface
/// was exactly that difference — so calling this and labelling the result with
/// a remote machine's name would manufacture the lie it is meant to expose.
///
/// ⚠ Deliberately a lookup and not an execution. Running `--version` to decide
/// presence costs a process per CLI per repaint, and for at least one vendor
/// CLI the first invocation unpacks a payload and writes over a hundred
/// megabytes — a probe that expensive changes the machine it is measuring.
pub fn probe_presence_with(
    descriptor: &AgentCliDescriptor,
    mut resolves: impl FnMut(&str) -> bool,
) -> CliPresence {
    if resolves(descriptor.binary_name) {
        CliPresence::Present { version: None }
    } else {
        CliPresence::Absent
    }
}

/// The whole matrix for the machine this process runs on, one row per
/// registered agent CLI, against the caller's resolver.
pub fn machine_status_with(
    display_label: impl Into<String>,
    mut resolves: impl FnMut(&str) -> bool,
) -> MachineCliStatus {
    MachineCliStatus::build("", display_label, |descriptor| {
        probe_presence_with(descriptor, &mut resolves)
    })
}

/// Can the CALLING PROCESS exec `binary` — a plain walk of its own `PATH`.
///
/// ⛔ **NOT launch parity, and never an answer to "can yggterm start this
/// here".** A daemon's `PATH` is whatever started it and a GUI's is whatever
/// the desktop session had; neither carries the managed CLI bin dir, and a
/// login shell's dirs reach both only by accident. Anything reporting what a
/// session will resolve must take the server crate's launch-parity resolver
/// instead. Kept public only so a caller that genuinely means "this process"
/// has to say so in the name.
pub fn binary_on_process_path(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable_file(&dir.join(binary)))
}


#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

/// One CLI's measured presence on one machine, as it crosses the wire.
///
/// ⛔ **ONLY THE MEASURED FACT TRAVELS.** Display name, install method, whether the
/// CLI is recommended — all of that is DERIVED from the registry by whoever receives
/// this, never sent. A remote host shipping its own idea of a CLI's display name would
/// be a second registry that can disagree with the first, which is the exact shape the
/// single-source-of-truth law forbids. What the remote knows and the local side cannot
/// is one bit: *is the binary there.*
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CliPresenceReport {
    pub slug: String,
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Probe every registered CLI on THIS machine and return the wire report.
///
/// Run on the remote host, by the remote host, against the resolution a LAUNCH
/// there would perform — not against whatever `PATH` the invoking `ssh` handed
/// the process. Those differ by more than a directory: a non-login `ssh` drops
/// the user's own bin dir, and no `ssh` at all carries the managed CLI bin dir
/// the launch prepends. Reporting the invoking `PATH` made the matrix advertise
/// installs for CLIs the machine already had and was already running.
pub fn presence_report_with(mut resolves: impl FnMut(&str) -> bool) -> Vec<CliPresenceReport> {
    AGENT_CLIS
        .iter()
        .filter(|descriptor| descriptor.slug != "shell")
        .map(|descriptor| {
            let presence = probe_presence_with(descriptor, &mut resolves);
            CliPresenceReport {
                slug: descriptor.slug.to_string(),
                present: presence.is_present(),
                version: match presence {
                    CliPresence::Present { version } => version,
                    _ => None,
                },
            }
        })
        .collect()
}

/// Rebuild a machine's matrix from a report a remote host sent back.
///
/// ⛔ **A slug the report does not mention stays `Unknown`, never `Absent`.** The two
/// cases it separates are "that host told us the binary is missing" and "that host is
/// running an older build that did not know to look" — and only the first is work.
/// Collapsing them would let a version skew read as a fleet-wide absence and offer
/// installs nobody asked for.
pub fn machine_status_from_report(
    machine_key: impl Into<String>,
    display_label: impl Into<String>,
    report: &[CliPresenceReport],
) -> MachineCliStatus {
    MachineCliStatus::build(machine_key, display_label, |descriptor| {
        match report.iter().find(|row| row.slug == descriptor.slug) {
            Some(row) if row.present => CliPresence::Present {
                version: row.version.clone(),
            },
            Some(_) => CliPresence::Absent,
            None => CliPresence::Unknown,
        }
    })
}

/// One machine's worth of work, as the modal's action button would perform it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InstallPlan {
    pub machine_key: String,
    pub slugs: Vec<&'static str>,
}

impl InstallPlan {
    pub fn is_empty(&self) -> bool {
        self.slugs.is_empty()
    }
}

/// THE DEFAULT RECOMMENDATION, in one function: **every CLI on every machine.**
///
/// Owner-settled (`docs/settled-calls.md`, 2026-08-08). Machines whose CLIs are
/// all present, or which have not been probed, contribute nothing — an empty
/// plan is the success case here, not a failure to find work.
///
/// ⛔ Returns an empty plan set when consent is not granted, rather than a plan
/// the caller might run anyway. The gate lives with the plan so a caller cannot
/// hold one without the other.
pub fn recommended_plans(
    machines: &[MachineCliStatus],
    consent: InstallConsent,
) -> Vec<InstallPlan> {
    if !consent.may_fetch() {
        return Vec::new();
    }
    machines
        .iter()
        .map(|machine| InstallPlan {
            machine_key: machine.machine_key.clone(),
            slugs: machine.installable().map(|row| row.slug).collect(),
        })
        .filter(|plan| !plan.is_empty())
        .collect()
}

/// How many individual installs a plan set would perform — what the modal's
/// primary button counts, so the number the user reads is the number that runs.
pub fn plan_install_count(plans: &[InstallPlan]) -> usize {
    plans.iter().map(|plan| plan.slugs.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(presences: &[(&str, CliPresence)]) -> MachineCliStatus {
        MachineCliStatus::build("box", "box", |descriptor| {
            presences
                .iter()
                .find(|(slug, _)| *slug == descriptor.slug)
                .map(|(_, presence)| presence.clone())
                .unwrap_or(CliPresence::Absent)
        })
    }

    #[test]
    fn an_absent_value_never_authorises_fetching() {
        // A corrupt or empty settings value must degrade to Undecided. If it
        // degraded to Granted, an unreadable file would install other people's
        // software on every machine.
        assert_eq!(InstallConsent::from_wire(""), InstallConsent::Undecided);
        assert_eq!(InstallConsent::from_wire("garbage"), InstallConsent::Undecided);
        assert!(!InstallConsent::from_wire("garbage").may_fetch());
        assert_eq!(InstallConsent::default(), InstallConsent::Undecided);
    }

    #[test]
    fn consent_round_trips_through_its_wire_word() {
        for consent in [
            InstallConsent::Undecided,
            InstallConsent::Granted,
            InstallConsent::Declined,
        ] {
            assert_eq!(InstallConsent::from_wire(consent.as_wire()), consent);
        }
    }

    #[test]
    fn a_decline_is_an_answer_and_stops_both_the_offer_and_the_fetch() {
        assert!(!InstallConsent::Declined.should_offer());
        assert!(!InstallConsent::Declined.may_fetch());
        assert!(InstallConsent::Undecided.should_offer());
        assert!(!InstallConsent::Undecided.may_fetch());
    }

    #[test]
    fn the_default_recommendation_is_every_cli_on_every_machine() {
        let machines = vec![status(&[]), {
            let mut other = status(&[]);
            other.machine_key = "other".to_string();
            other
        }];
        let plans = recommended_plans(&machines, InstallConsent::Granted);
        assert_eq!(plans.len(), 2, "both machines carry work");
        for plan in &plans {
            assert!(
                !plan.is_empty(),
                "every unattended CLI is recommended on every machine"
            );
        }
    }

    #[test]
    fn an_unprobed_machine_contributes_no_work() {
        // The trap this guards: rendering Unknown as Absent would make the
        // primary button claim installs it cannot perform, against a host it
        // could not even reach.
        let machine = MachineCliStatus::build("box", "box", |_| CliPresence::Unknown);
        assert_eq!(machine.installable().count(), 0);
        assert_eq!(machine.summary(), "not probed");
        assert!(recommended_plans(&[machine], InstallConsent::Granted).is_empty());
    }

    #[test]
    fn a_platform_that_cannot_run_a_cli_is_not_recommended_it() {
        let machine = MachineCliStatus::build("box", "box", |_| CliPresence::UnsupportedHere);
        assert!(machine.rows.iter().all(|row| !row.recommended));
        assert_eq!(machine.installable().count(), 0);
    }

    #[test]
    fn a_manual_cli_is_reported_as_needing_the_human_not_silently_dropped() {
        // Manual CLIs are still RECOMMENDED - the owner wants them everywhere -
        // but they can never appear in an unattended plan, or the plan would
        // promise an install that cannot happen.
        let machine = status(&[]);
        let manual: Vec<_> = machine.needing_human().map(|row| row.slug).collect();
        for slug in &manual {
            assert!(
                !machine.installable().any(|row| row.slug == *slug),
                "{slug} must not appear in an unattended plan"
            );
        }
        assert!(
            machine.rows.iter().any(|row| row.needs_human()) == !manual.is_empty(),
            "the two views agree"
        );
    }

    #[test]
    fn no_consent_means_no_plan_even_when_every_cli_is_missing() {
        let machine = status(&[]);
        assert!(machine.installable().count() > 0, "there IS work to do");
        assert!(recommended_plans(
            std::slice::from_ref(&machine),
            InstallConsent::Undecided
        )
        .is_empty());
        assert!(recommended_plans(&[machine], InstallConsent::Declined).is_empty());
    }

    #[test]
    fn the_counted_number_is_the_number_that_would_run() {
        let machine = status(&[]);
        let expected = machine.installable().count();
        let plans = recommended_plans(std::slice::from_ref(&machine), InstallConsent::Granted);
        assert_eq!(plan_install_count(&plans), expected);
    }

    #[test]
    fn a_fully_installed_machine_drops_out_of_the_plan_entirely() {
        let machine = MachineCliStatus::build("box", "box", |_| CliPresence::Present {
            version: Some("1.0.0".to_string()),
        });
        assert!(recommended_plans(&[machine.clone()], InstallConsent::Granted).is_empty());
        assert!(machine.summary().contains("installed"));
    }

    #[test]
    fn a_probe_reaches_a_verdict_for_every_registered_cli() {
        let status = machine_status_with("this machine", |binary| binary == "claude");
        assert_eq!(status.rows.len(), AGENT_CLIS.iter().filter(|d| d.slug != "shell").count());
        for row in &status.rows {
            assert!(
                matches!(row.presence, CliPresence::Present { .. } | CliPresence::Absent),
                "a probe always reaches a verdict; it never returns Unknown"
            );
        }
        assert_eq!(status.present_count(), 1, "only the resolvable binary is present");
    }

    #[test]
    fn the_injected_resolver_is_the_only_thing_consulted() {
        // ⛔ THE REGRESSION THIS EXISTS FOR. The probe used to walk the calling
        // process's own `PATH`, which made every remote machine report the
        // `PATH` its `ssh` happened to inherit rather than the one a launch
        // there will search — one CLI of ten on a machine that resolves all
        // ten. A resolver that says "nothing is here" must produce a report
        // that says nothing is here, no matter what the test machine has
        // installed, or the ambient answer has crept back in.
        let none = presence_report_with(|_| false);
        assert!(none.iter().all(|row| !row.present), "no ambient fallback");
        let all = presence_report_with(|_| true);
        assert!(all.iter().all(|row| row.present));
        assert_eq!(none.len(), all.len());
    }

    #[test]
    fn a_probe_never_claims_a_machine_key() {
        // The local status uses the empty machine key by convention. A local
        // probe labelled with a remote machine's key is the exact confusion
        // this module was written to prevent.
        assert_eq!(machine_status_with("here", |_| true).machine_key, "");
    }

    #[test]
    fn a_report_round_trips_into_the_same_matrix() {
        let resolves = |binary: &str| binary.starts_with('c');
        let report = presence_report_with(resolves);
        assert_eq!(report.len(), AGENT_CLIS.iter().filter(|d| d.slug != "shell").count());
        let json: Vec<String> = report
            .iter()
            .map(|row| serde_json::to_string(row).expect("serialises"))
            .collect();
        let back: Vec<CliPresenceReport> = json
            .iter()
            .map(|line| serde_json::from_str(line).expect("round-trips"))
            .collect();
        assert_eq!(report, back);
        let direct = machine_status_with("box", resolves);
        let rebuilt = machine_status_from_report("box", "box", &back);
        assert_eq!(
            direct.rows.iter().map(|r| (r.slug, r.presence.is_present())).collect::<Vec<_>>(),
            rebuilt.rows.iter().map(|r| (r.slug, r.presence.is_present())).collect::<Vec<_>>(),
            "a probe here and the same probe reported from there must agree"
        );
    }

    #[test]
    fn a_cli_missing_from_the_report_is_unknown_not_absent() {
        // The version-skew case: an older remote build reports only the CLIs it knew
        // about. The ones it never mentioned must not be counted as installable work.
        let partial = vec![CliPresenceReport {
            slug: "claude-code".to_string(),
            present: true,
            version: None,
        }];
        let status = machine_status_from_report("box", "box", &partial);
        let unknown = status
            .rows
            .iter()
            .filter(|row| matches!(row.presence, CliPresence::Unknown))
            .count();
        assert!(unknown > 0, "unmentioned CLIs stay Unknown");
        assert_eq!(status.installable().count(), 0, "Unknown is never work");
        assert!(recommended_plans(&[status], InstallConsent::Granted).is_empty());
    }

    #[test]
    fn an_empty_report_leaves_every_cli_unknown() {
        // What an unreachable host produces. It must read as "not probed", never as a
        // machine that is missing everything.
        let status = machine_status_from_report("box", "box", &[]);
        assert_eq!(status.summary(), "not probed");
        assert_eq!(status.installable().count(), 0);
    }

    #[test]
    fn every_machine_lists_the_clis_in_the_same_order() {
        let a = status(&[]);
        let b = MachineCliStatus::build("b", "b", |_| CliPresence::Present { version: None });
        let slugs_a: Vec<_> = a.rows.iter().map(|row| row.slug).collect();
        let slugs_b: Vec<_> = b.rows.iter().map(|row| row.slug).collect();
        assert_eq!(slugs_a, slugs_b, "a matrix whose rows reorder cannot be scanned");
        assert!(!slugs_a.is_empty());
        assert!(!slugs_a.contains(&"shell"), "a plain shell is not an agent CLI");
    }
}
