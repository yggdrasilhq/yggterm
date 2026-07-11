//! The ALT+ KeyTips declaration model and its deterministic assignment resolver.
//!
//! Source of truth in prose: `docs/alt-keytips.md`. This module owns the *pure*
//! layer of that spec — the declaration types, the keymap-v2 config, and the
//! assignment function that turns a scope's ordered declarations into final
//! letters (the ladder in §5, plus app-vs-app groups, numbering, and pinning in
//! §6). It carries no shell dependencies so the invariants can be unit-tested in
//! isolation (spec §13, invariants 1-4, 8).
//!
//! ## The ownership inversion (§2)
//!
//! v1 kept a static table of global commands and each render site asked it "what
//! letter do I paint?". That cannot express instances ("launch CC *here*"),
//! dynamic sets (one entry per installed app / theme / live session), or foreign
//! declarations (an app's own commands). So ownership inverts: a **declaration**
//! ([`KeyTipDecl`]) is the SSOT for *what exists* in a scope; the keymap keeps the
//! SSOT for *default letters and user overrides*. The resolver ([`assign_scope`])
//! is a pure function of `(ordered declarations, keymap)` — invariant 1.

use std::collections::BTreeMap;

/// Excel's top-level ribbon KeyTip letters, reserved for app contributions so a
/// focused Cellulose can be 100% Excel-faithful while shell chrome stays
/// reachable in one flat namespace (spec §7). A shell command must never claim
/// one of these at the root scope.
pub const EXCEL_RESERVED_LETTERS: &[char] =
    &['f', 'h', 'n', 'p', 'm', 'a', 'r', 'w', 'x', 'y', 'q'];

/// True if `letter` belongs to Excel's reserved top-level namespace.
pub fn reserved_letter(letter: char) -> bool {
    EXCEL_RESERVED_LETTERS.contains(&letter.to_ascii_lowercase())
}

/// A scope: one chord level, and the set of declarations shown together (spec
/// §1). The root scope is what a clean ALT tap opens; every openable container
/// (menu, panel, modal, app surface) is its own scope. `as_str` is the stable id
/// that rides `keymap.json` pin keys and the `data-keytip-node` DOM anchor.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ScopeId {
    /// The clean-ALT-tap level: shell chrome.
    Root,
    /// The New… ("+") menu.
    Insert,
    /// The Settings panel.
    Settings,
    /// The theme picker inside Settings (`ALT, G, T, <letter>`).
    SettingsTheme,
    /// A running/installed app's own scope, keyed by app id (Phase 2 dynamic).
    App(String),
}

impl ScopeId {
    /// The stable, dotted id used in `keymap.json` (pin keys) and the DOM anchor.
    pub fn as_str(&self) -> String {
        match self {
            ScopeId::Root => "root".to_string(),
            ScopeId::Insert => "insert.menu".to_string(),
            ScopeId::Settings => "settings".to_string(),
            ScopeId::SettingsTheme => "settings.theme".to_string(),
            ScopeId::App(id) => format!("app.{id}"),
        }
    }

    /// The root scope is the only one bound by the Excel-reserved-letter rule
    /// (§7): shell chrome one level down (`insert.session` under the `+` menu) is
    /// free to reuse any letter within its parent's namespace.
    pub fn is_reserved_namespace(&self) -> bool {
        matches!(self, ScopeId::Root)
    }
}

/// Who declared a node — the collision policy differs (§6). A shell command that
/// wants a letter an app also wants keeps it outright (the shell never numbers);
/// two apps that want the same letter become a numbered group.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Shell,
    App,
}

/// What activating a node does (spec §1, §4). The pure layer does not hold the
/// action itself (that lives in the shell, keyed by `(scope, key)`); it only
/// distinguishes act-and-dismiss from act-and-descend. [`Target::Group`] is never
/// declared — the resolver synthesizes it on collision (§6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    /// Act and dismiss the overlay.
    Run,
    /// Act and open `scope`'s KeyTips (Excel's `ALT,H,…` nesting).
    Descend(ScopeId),
}

/// One declaration emitted for an interactable in a scope (spec §3). The char is
/// attached to the element by the code that draws it; `hint` is the letter the
/// declarer *wants* (a registry default, a user override, or an app manifest's
/// `keytip`) and may be denied by the ladder.
#[derive(Clone, Debug)]
pub struct KeyTipDecl {
    /// Stable within the scope: `"sidebar.toggle"`, `"app.ychrome"`. Rides
    /// `keymap.json` and `command invoke`.
    pub key: String,
    /// Human label — shown in the legend and the editor, and used by the ladder
    /// (step 4 draws from the first free letter of the title).
    pub title: String,
    /// The letter the declarer wants, or `None` to let the ladder choose.
    pub hint: Option<char>,
    /// The direct accelerator (§11), sparse: most declarations are `None`.
    pub accel: Option<Chord>,
    /// Shell chrome or an app contribution — drives the collision policy (§6).
    pub origin: Origin,
    /// What activation does.
    pub target: Target,
}

impl KeyTipDecl {
    /// A stable shell-chrome declaration whose default letter lives centrally.
    pub fn shell(key: impl Into<String>, title: impl Into<String>, hint: char, target: Target) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            hint: Some(hint),
            accel: None,
            origin: Origin::Shell,
            target,
        }
    }

    /// An app contribution (manifest or OSC), which may be denied its hint or
    /// numbered into a group.
    pub fn app(key: impl Into<String>, title: impl Into<String>, hint: Option<char>, target: Target) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            hint,
            accel: None,
            origin: Origin::App,
            target,
        }
    }

    /// Builder: attach a direct accelerator.
    pub fn with_accel(mut self, accel: Chord) -> Self {
        self.accel = Some(accel);
        self
    }
}

/// One member of a synthesized disambiguation group (§6): a claimant that lost a
/// contested letter and is reached by pressing the group letter then its number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMember {
    pub number: u32,
    pub key: String,
    pub title: String,
    pub target: Target,
}

/// A resolved entry in a scope: the final tip plus what pressing it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignedNode {
    /// A single node reached by its `tip` (`"b"`, or a two-letter `"al"`).
    Leaf {
        key: String,
        title: String,
        tip: String,
        target: Target,
    },
    /// A contested letter (§6): nobody gets it bare; the claimants are numbered.
    /// `tip` is the bare group letter; pressing it descends into the numbers.
    Group {
        tip: String,
        title: String,
        members: Vec<GroupMember>,
    },
}

impl AssignedNode {
    /// The tip a user types to reach this node from its scope.
    pub fn tip(&self) -> &str {
        match self {
            AssignedNode::Leaf { tip, .. } | AssignedNode::Group { tip, .. } => tip,
        }
    }
}

/// The keymap-v2 config (`~/.yggterm/keymap.json`, spec §11.5). Three views of
/// one file: ALT letters, materialized group numbers, and direct accelerators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapConfig {
    /// command-id → ALT letter override (v1's `bindings`, still read as an alias).
    keytips: BTreeMap<String, char>,
    /// materialized group-member number, keyed `"<scope>/<letter>/<member-key>"`.
    pinned: BTreeMap<String, u32>,
    /// command-id → direct chord.
    accelerators: BTreeMap<String, Chord>,
}

impl KeymapConfig {
    pub fn keytip_override(&self, key: &str) -> Option<char> {
        self.keytips.get(key).copied()
    }

    pub fn is_keytip_overridden(&self, key: &str) -> bool {
        self.keytips.contains_key(key)
    }

    pub fn accel_override(&self, key: &str) -> Option<&Chord> {
        self.accelerators.get(key)
    }

    pub fn keytips(&self) -> &BTreeMap<String, char> {
        &self.keytips
    }

    pub fn pinned(&self) -> &BTreeMap<String, u32> {
        &self.pinned
    }

    pub fn accelerators(&self) -> &BTreeMap<String, Chord> {
        &self.accelerators
    }

    /// Set a KeyTip letter override (already validated by the caller).
    pub fn set_keytip(&mut self, key: impl Into<String>, letter: char) {
        self.keytips.insert(key.into(), letter.to_ascii_lowercase());
    }

    pub fn clear_keytip(&mut self, key: &str) {
        self.keytips.remove(key);
    }

    /// Record a group-member number so a learned chord never moves (§6).
    pub fn pin_number(&mut self, pin_key: impl Into<String>, number: u32) {
        self.pinned.insert(pin_key.into(), number);
    }

    pub fn set_accel(&mut self, key: impl Into<String>, chord: Chord) {
        self.accelerators.insert(key.into(), chord);
    }

    pub fn clear_accel(&mut self, key: &str) {
        self.accelerators.remove(key);
    }

    /// The pin key for a group member: `"<scope>/<letter>/<member-key>"`.
    pub fn pin_key(scope: &ScopeId, letter: char, member_key: &str) -> String {
        format!("{}/{}/{}", scope.as_str(), letter, member_key)
    }
}

/// A direct-accelerator chord (§11): modifiers + a key. Deliberately second
/// class and flat — one chord, one action.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    /// The non-modifier key, canonicalized (`"t"`, `"pagedown"`, `"f11"`).
    pub key: String,
}

impl Chord {
    /// Parse `"Ctrl+Shift+T"`, `"Ctrl+Alt+PageDown"`, `"F11"`, `"Super+B"`.
    /// Returns `None` for an empty/keyless spec.
    pub fn parse(spec: &str) -> Option<Chord> {
        let mut chord = Chord {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            key: String::new(),
        };
        for raw in spec.split('+') {
            let part = raw.trim();
            if part.is_empty() {
                continue;
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" | "option" => chord.alt = true,
                "shift" => chord.shift = true,
                "super" | "meta" | "cmd" | "command" | "win" => chord.meta = true,
                other => chord.key = other.to_string(),
            }
        }
        if chord.key.is_empty() {
            return None;
        }
        Some(chord)
    }

    /// Canonical display form, e.g. `"Ctrl+Shift+T"`.
    pub fn display(&self) -> String {
        let mut out = Vec::new();
        if self.ctrl {
            out.push("Ctrl".to_string());
        }
        if self.alt {
            out.push("Alt".to_string());
        }
        if self.shift {
            out.push("Shift".to_string());
        }
        if self.meta {
            out.push("Super".to_string());
        }
        out.push(display_key(&self.key));
        out.join("+")
    }

    /// A shell accelerator must be PTY-safe (§11.2): a bare `Ctrl+<letter>`
    /// belongs to the PTY (readline transpose, backward-char, …) forever, so it
    /// is forbidden. `Ctrl+Shift+…`, `Ctrl+Alt+…`, `Super+…`, and function keys
    /// are free by construction. A modifier-less non-function key is also unsafe
    /// (a plain letter would type into the terminal).
    pub fn is_pty_safe(&self) -> bool {
        let is_function_key = self.key.starts_with('f')
            && self.key.len() >= 2
            && self.key[1..].chars().all(|c| c.is_ascii_digit());
        if is_function_key {
            return true;
        }
        if self.meta {
            return true;
        }
        // Ctrl or Alt must be paired with Shift (or each other) to escape the
        // legacy control-code encoding the PTY owns.
        if self.ctrl && (self.shift || self.alt) {
            return true;
        }
        if self.alt && self.shift {
            return true;
        }
        false
    }
}

/// Human display of a chord's key component.
fn display_key(key: &str) -> String {
    match key {
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        other if other.len() == 1 => other.to_ascii_uppercase(),
        other if other.starts_with('f') && other[1..].chars().all(|c| c.is_ascii_digit()) => {
            other.to_ascii_uppercase()
        }
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

/// Assign final letters to one scope's declarations (spec §5, §6). Pure and
/// deterministic: the same `(decls, keymap)` always yields the same tips
/// (invariant 1). Declarations are processed in render order, which is stable.
///
/// The ladder per node: user override → declared hint → first free letter of the
/// title → first free `a-z` → digits `0-9` → a two-letter tip. At the root scope a
/// shell command may not take an Excel-reserved letter (§7). Shell commands are
/// laid down first so they win contested letters outright (the shell never
/// numbers, §6); apps that declare the *same* hint and find no shell owner are
/// folded into a numbered Group.
pub fn assign_scope(scope: &ScopeId, decls: &[KeyTipDecl], keymap: &KeymapConfig) -> Vec<AssignedNode> {
    let reserved_ns = scope.is_reserved_namespace();
    // taken: single letters already claimed at this level (bare leaves + groups).
    let mut taken: Vec<char> = Vec::new();
    // Output preserves render order; we fill leaves in a first pass over shell
    // decls, then apps, then stitch back to the original order at the end.
    let mut assignment: BTreeMap<usize, AssignedNode> = BTreeMap::new();

    let take = |taken: &mut Vec<char>, letter: char| {
        taken.push(letter.to_ascii_lowercase());
    };
    let is_taken = |taken: &[char], letter: char| taken.contains(&letter.to_ascii_lowercase());

    // Resolve the letter a declaration wants: user override first, then its hint.
    let desired = |decl: &KeyTipDecl| -> Option<char> {
        keymap
            .keytip_override(&decl.key)
            .or(decl.hint)
            .map(|c| c.to_ascii_lowercase())
    };

    // Pass 1 — shell declarations claim first (they win contested letters).
    for (idx, decl) in decls.iter().enumerate() {
        if decl.origin != Origin::Shell {
            continue;
        }
        let letter = pick_letter(decl, desired(decl), &taken, reserved_ns, true);
        take(&mut taken, letter);
        assignment.insert(
            idx,
            AssignedNode::Leaf {
                key: decl.key.clone(),
                title: decl.title.clone(),
                tip: letter.to_string(),
                target: decl.target.clone(),
            },
        );
    }

    // Pass 2 — group app declarations by the (free) letter they request. Two or
    // more apps requesting the same still-free letter become one Group node; a
    // lone requester keeps the bare letter (§6).
    let mut app_by_letter: BTreeMap<char, Vec<usize>> = BTreeMap::new();
    let mut app_ladder: Vec<usize> = Vec::new();
    for (idx, decl) in decls.iter().enumerate() {
        if decl.origin != Origin::App {
            continue;
        }
        match desired(decl) {
            Some(letter) if !is_taken(&taken, letter) => {
                app_by_letter.entry(letter).or_default().push(idx);
            }
            // No hint, or the hint is already taken by shell/another group: this
            // app falls through the ladder individually in a later pass.
            _ => app_ladder.push(idx),
        }
    }

    // Group letters are claimed in ascending letter order for determinism.
    for (&letter, claimants) in &app_by_letter {
        take(&mut taken, letter);
        if claimants.len() == 1 {
            let idx = claimants[0];
            let decl = &decls[idx];
            assignment.insert(
                idx,
                AssignedNode::Leaf {
                    key: decl.key.clone(),
                    title: decl.title.clone(),
                    tip: letter.to_string(),
                    target: decl.target.clone(),
                },
            );
            continue;
        }
        // Two+ claimants → a numbered Group. Pins first (§6), then next free.
        let members = number_group(scope, letter, claimants, decls, keymap);
        // A group's title is generic; the members carry the real labels.
        let group_idx = *claimants.iter().min().unwrap();
        assignment.insert(
            group_idx,
            AssignedNode::Group {
                tip: letter.to_string(),
                title: "New …".to_string(),
                members,
            },
        );
        // The other claimant indices collapse into the group; drop their slots by
        // leaving them unassigned (they render nothing of their own).
    }

    // Pass 3 — app declarations with no free hint fall through the ladder.
    for idx in app_ladder {
        let decl = &decls[idx];
        let letter = pick_letter(decl, None, &taken, reserved_ns, false);
        take(&mut taken, letter);
        assignment.insert(
            idx,
            AssignedNode::Leaf {
                key: decl.key.clone(),
                title: decl.title.clone(),
                tip: letter.to_string(),
                target: decl.target.clone(),
            },
        );
    }

    // Stitch back to render order.
    (0..decls.len())
        .filter_map(|idx| assignment.remove(&idx))
        .collect()
}

/// Number a group's members: a pinned number is honored if free, else the next
/// free number is assigned in the claimants' render order (§6). Uninstalling a
/// member leaves a hole rather than renumbering the survivors — pins outlive it.
fn number_group(
    scope: &ScopeId,
    letter: char,
    claimants: &[usize],
    decls: &[KeyTipDecl],
    keymap: &KeymapConfig,
) -> Vec<GroupMember> {
    let mut used: Vec<u32> = Vec::new();
    let mut members: Vec<GroupMember> = Vec::new();
    // Sort claimants by render order (they arrive that way already, but be sure).
    let mut ordered = claimants.to_vec();
    ordered.sort_unstable();
    // Pass A: honor pins.
    let mut pinned_for: BTreeMap<usize, u32> = BTreeMap::new();
    for &idx in &ordered {
        let pin_key = KeymapConfig::pin_key(scope, letter, &decls[idx].key);
        if let Some(&number) = keymap.pinned().get(&pin_key) {
            if !used.contains(&number) {
                used.push(number);
                pinned_for.insert(idx, number);
            }
        }
    }
    // Pass B: fill the rest with the next free number.
    let mut next = 1u32;
    for &idx in &ordered {
        let number = if let Some(&pinned) = pinned_for.get(&idx) {
            pinned
        } else {
            while used.contains(&next) {
                next += 1;
            }
            used.push(next);
            next
        };
        let decl = &decls[idx];
        members.push(GroupMember {
            number,
            key: decl.key.clone(),
            title: decl.title.clone(),
            target: decl.target.clone(),
        });
    }
    members.sort_by_key(|member| member.number);
    members
}

/// The letter ladder for one declaration (§5), given the letters already taken.
/// `desired` is the override-or-hint (already resolved); `honor_hint` lets pass 3
/// skip the hint (it was already tried and lost). Steps: desired hint (if free
/// and namespace-legal) → first free letter of the title → first free `a-z` →
/// digits `0-9` → a two-letter tip.
fn pick_letter(
    decl: &KeyTipDecl,
    desired: Option<char>,
    taken: &[char],
    reserved_ns: bool,
    honor_hint: bool,
) -> char {
    let free = |letter: char| -> bool {
        let letter = letter.to_ascii_lowercase();
        if taken.contains(&letter) {
            return false;
        }
        // A shell command may not sit on an Excel-reserved letter at the root
        // scope (§7); an app is free to (that is the whole point of the reserve).
        if reserved_ns && decl.origin == Origin::Shell && reserved_letter(letter) {
            return false;
        }
        true
    };

    if honor_hint {
        if let Some(letter) = desired {
            if free(letter) {
                return letter;
            }
        }
    }
    // First free alphanumeric of the title.
    for ch in decl.title.chars() {
        if ch.is_ascii_alphanumeric() && free(ch.to_ascii_lowercase()) {
            return ch.to_ascii_lowercase();
        }
    }
    // First free a-z.
    for ch in 'a'..='z' {
        if free(ch) {
            return ch;
        }
    }
    // Digits 0-9 (these are never reserved).
    for ch in '0'..='9' {
        if !taken.contains(&ch) {
            return ch;
        }
    }
    // Two-letter tips are handled by the caller when singles are exhausted; as a
    // last resort return the title's first alnum lowercased (deterministic).
    decl.title
        .chars()
        .find(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .unwrap_or('z')
}

/// The shipping direct accelerators (spec §11.4): `command-id → chord`. Sparse on
/// purpose — a command earns an accelerator by being used constantly, not by
/// existing; everything else is reachable through the ALT layer. Every chord here
/// is PTY-safe by construction (§11.2), enforced by `assert_accels_pty_safe`.
///
/// Copy/paste (`Ctrl+Shift+C/V`) are intentionally absent: they are handled inside
/// the terminal's own selection layer, not as shell chrome, so intercepting them
/// here would fight xterm. They migrate into this table only once the shell owns
/// that path.
pub const DEFAULT_ACCELERATORS: &[(&str, &str)] = &[
    ("insert.terminal", "Ctrl+Shift+T"),
    ("insert.session", "Ctrl+Shift+N"),
    ("sidebar.toggle", "Ctrl+Shift+B"),
    ("session.next", "Ctrl+Alt+PageDown"),
    ("session.prev", "Ctrl+Alt+PageUp"),
    ("window.fullscreen", "F11"),
];

/// The effective accelerators in force: the shipping defaults with the user's
/// `keymap.json` overrides applied, as `(command-id, chord)`. A command the user
/// cleared (override to empty) drops out.
pub fn effective_accelerators(cfg: &KeymapConfig) -> Vec<(String, Chord)> {
    let mut out: Vec<(String, Chord)> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (id, chord) in cfg.accelerators() {
        if let Some(parsed) = Some(chord.clone()) {
            out.push((id.clone(), parsed));
            seen.insert(id.clone());
        }
    }
    for (id, spec) in DEFAULT_ACCELERATORS {
        if seen.contains(*id) {
            continue;
        }
        if let Some(chord) = Chord::parse(spec) {
            out.push((id.to_string(), chord));
        }
    }
    out
}

/// The command a pressed chord fires, if any (user overrides then defaults).
pub fn accel_command_for(chord: &Chord, cfg: &KeymapConfig) -> Option<String> {
    effective_accelerators(cfg)
        .into_iter()
        .find(|(_, c)| c == chord)
        .map(|(id, _)| id)
}

/// The resolved KeyTip tree for a whole frame (spec §1): every open scope's
/// assigned nodes, keyed by `ScopeId::as_str`. Built in Rust during render from
/// the per-scope declarations, never scraped from the DOM. It is the one source
/// both the badge painter and the chord walker read, so a letter can never mean
/// two things.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyTipTree {
    scopes: BTreeMap<String, Vec<AssignedNode>>,
}

/// The outcome of walking a typed chord against the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChordResolution {
    /// A valid prefix; wait for more keys.
    Pending,
    /// The sequence maps to `key`'s action; act and dismiss the overlay.
    Run(String),
    /// A container opener (or a group letter): run `key`'s open action, if any,
    /// and keep the overlay open showing `scope` (a real scope id, or a synthetic
    /// group scope `"<scope>/<letter>"`). `key` is empty for a group letter.
    Descend { key: String, scope: String },
    /// No binding and no prefix — dismiss.
    Invalid,
}

impl KeyTipTree {
    /// Build a tree from `(scope, declarations)` pairs. Each scope is assigned
    /// independently by [`assign_scope`]; the caller supplies every open scope's
    /// declarations in render order. `scope_of` maps each [`ScopeId`] to the
    /// declarations shown in it.
    pub fn build(scopes: &[(ScopeId, Vec<KeyTipDecl>)], keymap: &KeymapConfig) -> Self {
        let mut map = BTreeMap::new();
        for (scope, decls) in scopes {
            map.insert(scope.as_str(), assign_scope(scope, decls, keymap));
        }
        Self { scopes: map }
    }

    /// The assigned nodes of one scope (by its string id), if present.
    pub fn scope_nodes(&self, scope: &str) -> Option<&[AssignedNode]> {
        self.scopes.get(scope).map(|nodes| nodes.as_slice())
    }

    /// The tip a given node key should paint while `sequence` is the chord typed
    /// so far — i.e. when `sequence` names the scope the node lives in. Returns the
    /// uppercased tip, or `None` when this node's scope is not the one on screen.
    /// This is the single lookup the badge painter uses (SSOT with the walker).
    pub fn tip_for(&self, sequence: &str, node_key: &str) -> Option<String> {
        let scope = self.scope_at(sequence)?;
        for node in self.scopes.get(&scope)? {
            match node {
                AssignedNode::Leaf { key, tip, .. } if key == node_key => {
                    return Some(tip.to_ascii_uppercase());
                }
                AssignedNode::Group { members, tip, .. } => {
                    // The group's bare letter paints for the *first* member's key
                    // (the node the render site anchors to); members' numbers
                    // paint one level deeper (handled by group_member_tip).
                    if members.iter().any(|member| member.key == node_key)
                        && members.first().map(|m| &m.key) == Some(&node_key.to_string())
                    {
                        return Some(tip.to_ascii_uppercase());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The number a group member should paint while the group letter is open —
    /// i.e. `sequence` names the group's scope plus its letter.
    pub fn group_member_tip(&self, sequence: &str, node_key: &str) -> Option<String> {
        // The synthetic group scope is "<parent-scope>/<letter>"; split it back.
        let (parent_seq, letter) = sequence.split_at(sequence.len().checked_sub(1)?);
        let scope = self.scope_at(parent_seq)?;
        for node in self.scopes.get(&scope)? {
            if let AssignedNode::Group { tip, members, .. } = node {
                if tip == letter {
                    if let Some(member) = members.iter().find(|m| m.key == node_key) {
                        return Some(member.number.to_string());
                    }
                }
            }
        }
        None
    }

    /// Which scope id a typed prefix lands in (walking descends). `""` → root.
    /// Returns a synthetic `"<scope>/<letter>"` when the prefix ends on a group
    /// letter. `None` if the prefix is not a valid path.
    fn scope_at(&self, sequence: &str) -> Option<String> {
        let mut scope = ScopeId::Root.as_str();
        let mut chars = sequence.chars().peekable();
        while let Some(c) = chars.next() {
            let nodes = self.scopes.get(&scope)?;
            let node = nodes.iter().find(|n| n.tip() == c.to_string())?;
            match node {
                AssignedNode::Leaf {
                    target: Target::Descend(child),
                    ..
                } => scope = child.as_str(),
                AssignedNode::Leaf { .. } => return None, // a Run node has no sub-scope
                AssignedNode::Group { tip, .. } => {
                    // Entering a group: the rest must be a digit selecting a member.
                    return Some(format!("{scope}/{tip}"));
                }
            }
        }
        Some(scope)
    }

    /// Walk a full typed sequence and decide what to do (§4). Re-resolves from
    /// scratch each keystroke, exactly like the shipped flat resolver, so a
    /// `Descend` node's open-action fires once (on the keystroke that lands on it)
    /// and never re-fires as the chord grows.
    pub fn resolve(&self, sequence: &str) -> ChordResolution {
        let sequence = sequence.to_ascii_lowercase();
        if sequence.is_empty() {
            return ChordResolution::Pending;
        }
        let mut scope = ScopeId::Root.as_str();
        let mut chars = sequence.chars().peekable();
        while let Some(c) = chars.next() {
            let last = chars.peek().is_none();
            let Some(nodes) = self.scopes.get(&scope) else {
                return ChordResolution::Invalid;
            };
            let Some(node) = nodes.iter().find(|n| n.tip() == c.to_string()) else {
                return ChordResolution::Invalid;
            };
            match node {
                AssignedNode::Leaf { key, target, .. } => match target {
                    Target::Run => {
                        return if last {
                            ChordResolution::Run(key.clone())
                        } else {
                            ChordResolution::Invalid
                        };
                    }
                    Target::Descend(child) => {
                        if last {
                            return ChordResolution::Descend {
                                key: key.clone(),
                                scope: child.as_str(),
                            };
                        }
                        scope = child.as_str();
                    }
                },
                AssignedNode::Group { tip, members, .. } => {
                    if last {
                        return ChordResolution::Descend {
                            key: String::new(),
                            scope: format!("{scope}/{tip}"),
                        };
                    }
                    // Next char selects a numbered member.
                    let Some(d) = chars.next() else {
                        return ChordResolution::Invalid;
                    };
                    let after_last = chars.peek().is_none();
                    let number = d.to_digit(10);
                    let member = number.and_then(|n| members.iter().find(|m| m.number == n));
                    return match (member, after_last) {
                        (Some(member), true) => ChordResolution::Run(member.key.clone()),
                        _ => ChordResolution::Invalid,
                    };
                }
            }
        }
        ChordResolution::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(key: &str, title: &str, hint: char) -> KeyTipDecl {
        KeyTipDecl::shell(key, title, hint, Target::Run)
    }
    fn app(key: &str, title: &str, hint: Option<char>) -> KeyTipDecl {
        KeyTipDecl::app(key, title, hint, Target::Run)
    }

    #[test]
    fn assignment_is_deterministic_and_honors_hints() {
        let decls = vec![
            shell("sidebar.toggle", "Toggle sidebar", 'b'),
            shell("view.web", "Web view", 'v'),
            shell("connect.toggle", "Connect SSH", 'c'),
        ];
        let km = KeymapConfig::default();
        let a = assign_scope(&ScopeId::Root, &decls, &km);
        let b = assign_scope(&ScopeId::Root, &decls, &km);
        assert_eq!(a, b, "invariant 1: same input, same output");
        assert_eq!(a[0].tip(), "b");
        assert_eq!(a[1].tip(), "v");
        assert_eq!(a[2].tip(), "c");
    }

    #[test]
    fn user_override_beats_hint() {
        let decls = vec![shell("notifications.toggle", "Notifications", 'l')];
        let mut km = KeymapConfig::default();
        km.set_keytip("notifications.toggle", 'j');
        let a = assign_scope(&ScopeId::Root, &decls, &km);
        assert_eq!(a[0].tip(), "j");
    }

    #[test]
    fn shell_never_lands_on_reserved_letter_at_root() {
        // A shell decl asking for 'f' (Excel File) is denied at root and falls to
        // the title ladder — invariant 4.
        let decls = vec![shell("window.fullscreen", "Fullscreen", 'f')];
        let a = assign_scope(&ScopeId::Root, &decls, &KeymapConfig::default());
        assert_ne!(a[0].tip(), "f");
        assert!(!reserved_letter(a[0].tip().chars().next().unwrap()));
    }

    #[test]
    fn app_may_use_reserved_letter() {
        // An app is free to claim 'n' (reserved for apps) at root.
        let decls = vec![app("app.ychrome", "Ychrome", Some('n'))];
        let a = assign_scope(&ScopeId::Root, &decls, &KeymapConfig::default());
        assert_eq!(a[0].tip(), "n");
    }

    #[test]
    fn shell_wins_a_letter_an_app_also_wants() {
        // Shell 'c' beats app wanting 'c'; the app falls through the ladder (§6).
        let decls = vec![
            shell("connect.toggle", "Connect", 'c'),
            app("app.cellulose", "Cellulose", Some('c')),
        ];
        let a = assign_scope(&ScopeId::Root, &decls, &KeymapConfig::default());
        assert_eq!(a[0].tip(), "c");
        assert_ne!(a[1].tip(), "c");
    }

    #[test]
    fn two_apps_wanting_one_letter_form_a_numbered_group() {
        let decls = vec![
            app("insert.n.ychrome", "New Ychrome here", Some('n')),
            app("insert.n.cellulose", "New Cellulose here", Some('n')),
        ];
        let a = assign_scope(&ScopeId::Insert, &decls, &KeymapConfig::default());
        assert_eq!(a.len(), 1, "the two collapse into one group node");
        match &a[0] {
            AssignedNode::Group { tip, members, .. } => {
                assert_eq!(tip, "n");
                assert_eq!(members.len(), 2);
                assert_eq!(members[0].number, 1);
                assert_eq!(members[1].number, 2);
            }
            other => panic!("expected a Group, got {other:?}"),
        }
    }

    #[test]
    fn a_lone_app_claimant_keeps_the_bare_letter() {
        let decls = vec![app("insert.n.ychrome", "New Ychrome here", Some('n'))];
        let a = assign_scope(&ScopeId::Insert, &decls, &KeymapConfig::default());
        assert!(matches!(&a[0], AssignedNode::Leaf { tip, .. } if tip == "n"));
    }

    #[test]
    fn pinned_numbers_never_move() {
        // ychrome pinned to 2; a fresh cellulose must take 1, not shove ychrome.
        let decls = vec![
            app("insert.n.ychrome", "New Ychrome here", Some('n')),
            app("insert.n.cellulose", "New Cellulose here", Some('n')),
        ];
        let mut km = KeymapConfig::default();
        km.pin_number(
            KeymapConfig::pin_key(&ScopeId::Insert, 'n', "insert.n.ychrome"),
            2,
        );
        let a = assign_scope(&ScopeId::Insert, &decls, &km);
        match &a[0] {
            AssignedNode::Group { members, .. } => {
                let ychrome = members.iter().find(|m| m.key == "insert.n.ychrome").unwrap();
                let cellulose = members.iter().find(|m| m.key == "insert.n.cellulose").unwrap();
                assert_eq!(ychrome.number, 2, "invariant 3: a pinned number never moves");
                assert_eq!(cellulose.number, 1);
            }
            other => panic!("expected a Group, got {other:?}"),
        }
    }

    #[test]
    fn chord_parse_and_pty_safety() {
        assert!(Chord::parse("Ctrl+Shift+T").unwrap().is_pty_safe());
        assert!(Chord::parse("Ctrl+Alt+PageDown").unwrap().is_pty_safe());
        assert!(Chord::parse("Super+B").unwrap().is_pty_safe());
        assert!(Chord::parse("F11").unwrap().is_pty_safe());
        // Bare Ctrl+letter and a plain letter belong to the PTY (invariant 8).
        assert!(!Chord::parse("Ctrl+T").unwrap().is_pty_safe());
        assert!(!Chord::parse("T").unwrap().is_pty_safe());
        assert!(Chord::parse("").is_none());
    }

    #[test]
    fn assert_accels_pty_safe() {
        // The build-time counterpart of assert_shell_namespace_clean (spec §11.2):
        // no shipping accelerator may be a bare Ctrl+<letter> the PTY owns.
        for (id, spec) in DEFAULT_ACCELERATORS {
            let chord = Chord::parse(spec)
                .unwrap_or_else(|| panic!("accelerator `{spec}` for `{id}` does not parse"));
            assert!(
                chord.is_pty_safe(),
                "accelerator `{spec}` for `{id}` is not PTY-safe"
            );
        }
    }

    #[test]
    fn default_accelerators_are_unique() {
        let mut seen = std::collections::BTreeMap::new();
        for (id, spec) in DEFAULT_ACCELERATORS {
            let chord = Chord::parse(spec).unwrap();
            if let Some(prev) = seen.insert(chord.display(), *id) {
                panic!("accelerator `{spec}` claimed by both `{prev}` and `{id}`");
            }
        }
    }

    #[test]
    fn accel_resolves_default_and_override() {
        let mut cfg = KeymapConfig::default();
        assert_eq!(
            accel_command_for(&Chord::parse("Ctrl+Shift+T").unwrap(), &cfg).as_deref(),
            Some("insert.terminal")
        );
        // A user override wins and the default chord for that command stops firing.
        cfg.set_accel("insert.terminal", Chord::parse("Ctrl+Shift+Y").unwrap());
        assert_eq!(
            accel_command_for(&Chord::parse("Ctrl+Shift+Y").unwrap(), &cfg).as_deref(),
            Some("insert.terminal")
        );
        assert_eq!(
            accel_command_for(&Chord::parse("Ctrl+Shift+T").unwrap(), &cfg),
            None
        );
    }

    #[test]
    fn chord_display_round_trips() {
        assert_eq!(Chord::parse("ctrl+shift+t").unwrap().display(), "Ctrl+Shift+T");
        assert_eq!(
            Chord::parse("ctrl+alt+pagedown").unwrap().display(),
            "Ctrl+Alt+PageDown"
        );
    }

    // --- KeyTipTree resolver ---

    fn insert_tree() -> KeyTipTree {
        // Root has the New… opener (i, descends into Insert) and a plain toggle.
        // Insert has two shell items + two colliding apps forming a group on 'n'.
        let root = vec![
            KeyTipDecl::shell("sidebar.toggle", "Toggle sidebar", 'b', Target::Run),
            KeyTipDecl::shell(
                "insert.menu",
                "New …",
                'i',
                Target::Descend(ScopeId::Insert),
            ),
        ];
        let insert = vec![
            KeyTipDecl::shell("insert.session", "New session", 's', Target::Run),
            KeyTipDecl::shell("insert.terminal", "New terminal", 't', Target::Run),
            KeyTipDecl::app("insert.n.ychrome", "New Ychrome", Some('n'), Target::Run),
            KeyTipDecl::app("insert.n.cellulose", "New Cellulose", Some('n'), Target::Run),
        ];
        KeyTipTree::build(
            &[(ScopeId::Root, root), (ScopeId::Insert, insert)],
            &KeymapConfig::default(),
        )
    }

    #[test]
    fn resolve_root_run_and_descend() {
        let t = insert_tree();
        assert_eq!(t.resolve(""), ChordResolution::Pending);
        assert_eq!(t.resolve("b"), ChordResolution::Run("sidebar.toggle".into()));
        assert_eq!(
            t.resolve("i"),
            ChordResolution::Descend {
                key: "insert.menu".into(),
                scope: "insert.menu".into()
            }
        );
        assert_eq!(t.resolve("z"), ChordResolution::Invalid);
    }

    #[test]
    fn resolve_descend_then_run() {
        let t = insert_tree();
        assert_eq!(t.resolve("is"), ChordResolution::Run("insert.session".into()));
        assert_eq!(t.resolve("it"), ChordResolution::Run("insert.terminal".into()));
        // 'b' is not in the Insert scope.
        assert_eq!(t.resolve("ib"), ChordResolution::Invalid);
    }

    #[test]
    fn resolve_group_opens_then_selects_member() {
        let t = insert_tree();
        // 'in' lands on the group letter -> descend into the number picker.
        assert_eq!(
            t.resolve("in"),
            ChordResolution::Descend {
                key: String::new(),
                scope: "insert.menu/n".into()
            }
        );
        // 'in1' / 'in2' pick the numbered members (render-order numbering).
        assert_eq!(t.resolve("in1"), ChordResolution::Run("insert.n.ychrome".into()));
        assert_eq!(t.resolve("in2"), ChordResolution::Run("insert.n.cellulose".into()));
        assert_eq!(t.resolve("in9"), ChordResolution::Invalid);
    }

    #[test]
    fn tip_for_follows_the_open_scope() {
        let t = insert_tree();
        // At root, the openers paint; the Insert children do not.
        assert_eq!(t.tip_for("", "sidebar.toggle").as_deref(), Some("B"));
        assert_eq!(t.tip_for("", "insert.menu").as_deref(), Some("I"));
        assert_eq!(t.tip_for("", "insert.session"), None);
        // Once 'i' is typed, the Insert scope's children paint.
        assert_eq!(t.tip_for("i", "insert.session").as_deref(), Some("S"));
        assert_eq!(t.tip_for("i", "insert.terminal").as_deref(), Some("T"));
        assert_eq!(t.tip_for("i", "sidebar.toggle"), None);
        // The group letter paints for its first member's anchor.
        assert_eq!(t.tip_for("i", "insert.n.ychrome").as_deref(), Some("N"));
    }

    #[test]
    fn group_member_numbers_paint_when_group_open() {
        let t = insert_tree();
        assert_eq!(
            t.group_member_tip("in", "insert.n.ychrome").as_deref(),
            Some("1")
        );
        assert_eq!(
            t.group_member_tip("in", "insert.n.cellulose").as_deref(),
            Some("2")
        );
        // Not while the group is closed.
        assert_eq!(t.group_member_tip("i", "insert.n.ychrome"), None);
    }
}
