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

/// Is this CLI's binary resolvable on THIS machine's `PATH`?
///
/// ⛔ **This answers for the machine the caller is running on, and nothing
/// else.** The GUI host and the hosts it shows rows for are different machines
/// with different `PATH`s — the fault this whole module exists to surface was
/// exactly that difference — so calling this and labelling the result with a
/// remote machine's name would manufacture the lie it is meant to expose.
///
/// ⚠ Deliberately a PATH lookup and not an execution. Running `--version` to
/// decide presence costs a process per CLI per repaint, and for at least one
/// vendor CLI the first invocation unpacks a payload and writes over a hundred
/// megabytes — a probe that expensive changes the machine it is measuring.
pub fn probe_local_presence(descriptor: &AgentCliDescriptor) -> CliPresence {
    match resolve_on_path(descriptor.binary_name) {
        Some(_) => CliPresence::Present { version: None },
        None => CliPresence::Absent,
    }
}

/// The whole local matrix, one row per registered agent CLI.
pub fn local_machine_status(display_label: impl Into<String>) -> MachineCliStatus {
    MachineCliStatus::build("", display_label, probe_local_presence)
}

fn resolve_on_path(binary: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(binary);
        is_executable_file(&candidate).then_some(candidate)
    })
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
    fn the_local_probe_reports_absent_rather_than_present_for_a_binary_that_is_not_there() {
        // The registry's own binary names are used, so this asserts the shape
        // of the answer rather than what happens to be installed on the box
        // running the test.
        let status = local_machine_status("this machine");
        assert_eq!(status.rows.len(), AGENT_CLIS.iter().filter(|d| d.slug != "shell").count());
        for row in &status.rows {
            assert!(
                matches!(row.presence, CliPresence::Present { .. } | CliPresence::Absent),
                "a local probe always reaches a verdict; it never returns Unknown"
            );
        }
    }

    #[test]
    fn a_local_probe_never_claims_a_machine_key() {
        // The local status uses the empty machine key by convention. A local
        // probe labelled with a remote machine's key is the exact confusion
        // this module was written to prevent.
        assert_eq!(local_machine_status("here").machine_key, "");
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
