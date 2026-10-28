//! The one command contract: loading, validation and structural checks.
//!
//! The JSON document owns grammar, examples, components and roles. This
//! module mirrors the loader contract of
//! `tools/swarmharness/introspection/commands.py` so both front ends reject
//! the same malformed generations. An installed contract file that exists
//! but fails validation is a hard refusal — never a silent fallback to the
//! embedded copy, which could pair new bytes with old expectations.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SUPPORTED_SCHEMA: u32 = 1;
pub const CONTRACT_NAME: &str = "ygg4-cli";

/// Embedded at build time from the repo SSOT; a standalone binary always
/// has a contract even before any payload install.
pub const EMBEDDED_IR: &str = include_str!("../../../tools/swarmharness/introspection/commands.json");

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Frontend {
    #[serde(default)]
    pub supported_schema: Vec<u32>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Component {
    pub runner: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub resolution: Vec<String>,
    #[serde(default)]
    pub interpreter: Option<String>,
    #[serde(default)]
    pub owner_managed: bool,
    #[serde(default)]
    pub owns: Vec<String>,
    #[serde(default)]
    pub reads: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct OptionEntry {
    #[serde(rename = "$ref", default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub int_range: Vec<i64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ExampleEntry {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub tags: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Record {
    pub path: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub effect: String,
    #[serde(default = "default_handler")]
    pub handler: String,
    #[serde(default)]
    pub options: Vec<OptionEntry>,
    #[serde(default)]
    pub operand: String,
    #[serde(default)]
    pub optional_operand: bool,
    #[serde(default)]
    pub examples: Vec<ExampleEntry>,
    #[serde(default)]
    pub legacy: Vec<String>,
    // ⛔ NOT bool's natural default: an omitted `strict` means strict, like
    // the Python dataclass default. Serde's `#[serde(default)]` would give
    // false and silently turn compatibility leaves into unchecked
    // passthroughs.
    #[serde(default = "default_true")]
    pub strict: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub component: String,
    #[serde(default)]
    pub operands: Vec<String>,
    #[serde(default)]
    pub passthrough: bool,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub native: String,
    #[serde(default)]
    pub require_one_of: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<Vec<String>>,
    #[serde(default)]
    pub operand_choices: Vec<String>,
    #[serde(default)]
    pub required_options: Vec<String>,
    #[serde(rename = "alias_of", default)]
    pub alias_of: Vec<String>,
}

fn default_handler() -> String {
    "group".into()
}

fn default_true() -> bool {
    true
}

impl Default for Record {
    fn default() -> Self {
        Record {
            path: Vec::new(),
            summary: String::new(),
            effect: String::new(),
            handler: default_handler(),
            options: Vec::new(),
            operand: String::new(),
            optional_operand: false,
            examples: Vec::new(),
            legacy: Vec::new(),
            strict: true,
            hidden: false,
            component: String::new(),
            operands: Vec::new(),
            passthrough: false,
            role: String::new(),
            native: String::new(),
            require_one_of: Vec::new(),
            conflicts: Vec::new(),
            operand_choices: Vec::new(),
            required_options: Vec::new(),
            alias_of: Vec::new(),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct Document {
    pub schema_version: u32,
    #[serde(default)]
    pub contract: String,
    #[serde(default)]
    pub frontend: Frontend,
    #[serde(default)]
    pub wired_adapters: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
    pub option_sets: BTreeMap<String, Vec<OptionEntry>>,
    pub commands: Vec<Record>,
}

/// A fully resolved command node: aliases flattened, `$ref` option sets
/// expanded, ready for grammar and dispatch without re-reading the document.
#[derive(Debug, Clone)]
pub struct Node {
    pub record: Record,
    pub options: Vec<OptionEntry>,
}

pub struct Loaded {
    pub document: Document,
    /// path -> resolved node
    pub tree: BTreeMap<Vec<String>, Node>,
    /// sha256 hex of the exact loaded bytes (the generation identity)
    pub contract_hash: String,
    /// where the bytes came from
    pub source: ContractSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContractSource {
    Embedded,
    File(PathBuf),
}

#[derive(Debug)]
pub enum LoadError {
    /// A contract the caller pointed at (env or installed payload) is
    /// unusable. Message text is user-facing; the exit code is 3.
    Refused(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Refused(m) => write!(f, "{m}"),
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// Structural validation shared with the Python loader.
pub fn parse_and_validate(raw: &str) -> Result<Document, LoadError> {
    let document: Document = serde_json::from_str(raw).map_err(|e| {
        LoadError::Refused(format!(
            "command contract unreadable ({e}); install a matching generation"
        ))
    })?;
    if document.schema_version != SUPPORTED_SCHEMA {
        return Err(LoadError::Refused(format!(
            "unsupported command contract schema {} (this frontend supports {SUPPORTED_SCHEMA}); install a matching generation",
            document.schema_version
        )));
    }
    let families: std::collections::BTreeSet<&str> = document
        .commands
        .iter()
        .filter_map(|r| r.path.first().map(|s| s.as_str()))
        .collect();
    for (name, spec) in &document.wired_adapters {
        if !families.contains(name.as_str()) {
            return Err(LoadError::Refused(format!(
                "wired adapter names an unknown family: {name}"
            )));
        }
        let adapter = spec.get("adapter").and_then(|v| v.as_str()).unwrap_or("");
        if adapter.is_empty()
            || adapter.contains('/')
            || adapter.contains('\\')
            || adapter.contains("..")
        {
            return Err(LoadError::Refused(format!(
                "wired adapter for {name} must name a bare adapter filename"
            )));
        }
    }
    let mut definitions: std::collections::BTreeSet<Vec<String>> = Default::default();
    for record in &document.commands {
        // The root record is the empty path; only empty SEGMENTS are
        // invalid. (The Python loader accepts the root the same way; the
        // previous `path.is_empty()` arm here refused the whole contract.)
        if record.path.iter().any(|p| p.is_empty()) {
            return Err(LoadError::Refused("invalid/duplicate command path".into()));
        }
        if !definitions.insert(record.path.clone()) {
            return Err(LoadError::Refused("invalid/duplicate command path".into()));
        }
        if !matches!(record.role.as_str(), "" | "planned") {
            return Err(LoadError::Refused(format!(
                "unsupported command role: {}",
                record.role
            )));
        }
        if !record.component.is_empty() {
            if !document.components.contains_key(&record.component) {
                return Err(LoadError::Refused(format!(
                    "command names an undeclared component: {}",
                    record.path.join("/")
                )));
            }
            if record.handler == "legacy" {
                let target = record
                    .legacy
                    .first()
                    .map(|s| s.trim_start_matches('@'))
                    .unwrap_or("");
                let declared = &document.components[&record.component];
                let roster: std::collections::BTreeSet<&str> =
                    std::iter::once(declared.runner.as_str())
                        .chain(declared.aliases.iter().map(|s| s.as_str()))
                        .collect();
                if !roster.contains(target) {
                    return Err(LoadError::Refused(format!(
                        "command target is not owned by its component: {}",
                        record.path.join("/")
                    )));
                }
            }
        }
    }
    Ok(document)
}

/// Expand `$ref` option sets with cycle detection (mirrors commands.py).
fn expand_options(
    document: &Document,
    entries: &[OptionEntry],
    stack: &[String],
) -> Result<Vec<OptionEntry>, LoadError> {
    let mut out = Vec::new();
    for entry in entries {
        if let Some(key) = &entry.reference {
            if stack.contains(key) || !document.option_sets.contains_key(key) {
                return Err(LoadError::Refused("cyclic/unknown option set".into()));
            }
            let mut next = stack.to_vec();
            next.push(key.clone());
            out.extend(expand_options(document, &document.option_sets[key], &next)?);
        } else {
            out.push(entry.clone());
        }
    }
    Ok(out)
}

/// Own fields override the alias base's. The contract currently declares no
/// alias records; this exists for forward compatibility and approximates
/// commands.py's "present keys win" by treating a field equal to the
/// default as absent — indistinguishable in JSON round-trip without
/// retaining declared-key sets, and irrelevant while no aliases ship.
fn overlay(base: Record, own: &Record) -> Record {
    let d = Record::default();
    let mut out = base;
    if own.summary != d.summary {
        out.summary = own.summary.clone();
    }
    if own.effect != d.effect {
        out.effect = own.effect.clone();
    }
    if own.handler != d.handler {
        out.handler = own.handler.clone();
    }
    if !own.options.is_empty() {
        out.options = own.options.clone();
    }
    if own.operand != d.operand {
        out.operand = own.operand.clone();
    }
    if own.optional_operand != d.optional_operand {
        out.optional_operand = own.optional_operand;
    }
    if !own.examples.is_empty() {
        out.examples = own.examples.clone();
    }
    if !own.legacy.is_empty() {
        out.legacy = own.legacy.clone();
    }
    if own.strict != d.strict {
        out.strict = own.strict;
    }
    if own.hidden != d.hidden {
        out.hidden = own.hidden;
    }
    if own.component != d.component {
        out.component = own.component.clone();
    }
    if !own.operands.is_empty() {
        out.operands = own.operands.clone();
    }
    if own.passthrough != d.passthrough {
        out.passthrough = own.passthrough;
    }
    if own.role != d.role {
        out.role = own.role.clone();
    }
    if own.native != d.native {
        out.native = own.native.clone();
    }
    if !own.require_one_of.is_empty() {
        out.require_one_of = own.require_one_of.clone();
    }
    if !own.conflicts.is_empty() {
        out.conflicts = own.conflicts.clone();
    }
    if !own.operand_choices.is_empty() {
        out.operand_choices = own.operand_choices.clone();
    }
    if !own.required_options.is_empty() {
        out.required_options = own.required_options.clone();
    }
    out
}

fn resolve_tree_node(
    document: &Document,
    tree: &mut BTreeMap<Vec<String>, Node>,
    definitions: &BTreeMap<Vec<String>, &Record>,
    path: &[String],
    stack: &mut Vec<Vec<String>>,
) -> Result<(), LoadError> {
    let key = path.to_vec();
    if tree.contains_key(&key) {
        return Ok(());
    }
    if stack.contains(&key) || !definitions.contains_key(&key) {
        return Err(LoadError::Refused("cyclic/unknown command alias".into()));
    }
    let record = definitions[&key];
    let mut effective = match () {
        _ if !record.alias_of.is_empty() => {
            stack.push(key.clone());
            resolve_tree_node(document, tree, definitions, &record.alias_of, stack)?;
            stack.pop();
            if !tree.contains_key(&record.alias_of) {
                return Err(LoadError::Refused("cyclic/unknown command alias".into()));
            }
            overlay(tree[&record.alias_of].record.clone(), record)
        }
        _ => record.clone(),
    };
    // A record's own non-empty `options` replace the base's; absence keeps
    // the base's expanded set, matching `if "options" in record` in
    // commands.py for every record this contract actually ships.
    let options = if !record.options.is_empty() {
        expand_options(document, &record.options, &[])?
    } else {
        effective.options.clone()
    };
    effective.options = options.clone();
    tree.insert(key, Node { record: effective, options });
    Ok(())
}

/// Load the contract: explicit env file, installed payload copy, then the
/// embedded SSOT bytes. A candidate that exists but is unusable is a hard
/// refusal; absence falls through to the next candidate — except an
/// explicitly named env file, whose absence is itself a refusal.
pub fn load() -> Result<Loaded, LoadError> {
    if let Ok(path) = std::env::var("YGG_COMMAND_IR") {
        if !path.trim().is_empty() {
            let path = PathBuf::from(path);
            let bytes = std::fs::read(&path).map_err(|e| {
                LoadError::Refused(format!(
                    "YGG_COMMAND_IR contract at {} unreadable ({}); install a matching generation",
                    path.display(),
                    e
                ))
            })?;
            let raw = String::from_utf8(bytes).map_err(|_| {
                LoadError::Refused(format!(
                    "YGG_COMMAND_IR contract at {} is not UTF-8; install a matching generation",
                    path.display()
                ))
            })?;
            return finish(raw, ContractSource::File(path));
        }
    }
    if let Some(dir) = payload_dir() {
        let path = dir.join("commands.json");
        if let Ok(bytes) = std::fs::read(&path) {
            let raw = String::from_utf8(bytes).map_err(|_| {
                LoadError::Refused(format!(
                    "command contract at {} is not UTF-8; install a matching generation",
                    path.display()
                ))
            })?;
            return finish(raw, ContractSource::File(path));
        }
    }
    finish(EMBEDDED_IR.to_string(), ContractSource::Embedded)
}

pub(crate) fn finish(raw: String, source: ContractSource) -> Result<Loaded, LoadError> {
    let contract_hash = sha256_hex(raw.as_bytes());
    let document = parse_and_validate(&raw)?;
    let mut definitions: BTreeMap<Vec<String>, &Record> = BTreeMap::new();
    for record in &document.commands {
        definitions.insert(record.path.clone(), record);
    }
    let mut tree: BTreeMap<Vec<String>, Node> = BTreeMap::new();
    for path in definitions.keys() {
        resolve_tree_node(&document, &mut tree, &definitions, path, &mut Vec::new())?;
    }
    if !tree.contains_key(&Vec::new()) {
        return Err(LoadError::Refused(
            "command contract has missing parent".into(),
        ));
    }
    for path in tree.keys() {
        if path.is_empty() {
            continue;
        }
        if !tree.contains_key(&path[..path.len() - 1].to_vec()) {
            return Err(LoadError::Refused(
                "command contract has missing parent".into(),
            ));
        }
    }
    Ok(Loaded {
        document,
        tree,
        contract_hash,
        source,
    })
}

/// The dispersed payload directory: `$YGG_BIN_DIR` or `~/.yggterm/bin/ygg`.
pub fn payload_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("YGG_BIN_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .map(|h| Path::new(&h).join(".yggterm/bin/ygg"))
}

/// The installed generation's declared layout under the payload root:
/// `commands.json` at the root, components under `components/`, the kernel
/// under `kernel/`. Declared in docs/ygg4-cli-contract.md; the packager
/// owns creating these, the resolver only reads them.
pub fn kernel_dir() -> PathBuf {
    payload_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kernel")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_contract_validates_and_builds_a_parent_complete_tree() {
        let loaded = load().expect("embedded contract must load");
        assert_eq!(loaded.source, ContractSource::Embedded);
        assert!(loaded.tree.contains_key(&Vec::new()));
        for path in loaded.tree.keys() {
            if path.is_empty() {
                continue;
            }
            assert!(
                loaded.tree.contains_key(&path[..path.len() - 1].to_vec()),
                "missing parent for {path:?}"
            );
        }
    }

    #[test]
    fn strict_defaults_to_true_for_omitted_strict_fields() {
        let document = parse_and_validate(EMBEDDED_IR).expect("valid");
        let status = document
            .commands
            .iter()
            .find(|r| r.path == vec!["dream", "status"])
            .expect("dream status exists");
        assert!(status.strict, "omitted strict must mean strict");
        let memory = document
            .commands
            .iter()
            .find(|r| r.path == vec!["memory"])
            .expect("memory exists");
        assert!(!memory.strict, "declared false must stay false");
    }

    #[test]
    fn every_legacy_record_is_component_owned() {
        let document = parse_and_validate(EMBEDDED_IR).expect("valid");
        for record in &document.commands {
            if record.handler == "legacy" {
                assert!(!record.component.is_empty(), "{:?}", record.path);
                assert!(document.components.contains_key(&record.component));
            }
        }
    }

    #[test]
    fn undeclared_component_is_refused() {
        let broken = EMBEDDED_IR.replace("\"component\": \"ci\"", "\"component\": \"ghost_plane\"");
        assert!(matches!(
            parse_and_validate(&broken),
            Err(LoadError::Refused(message)) if message.contains("undeclared component")
        ));
    }

    #[test]
    fn unowned_target_is_refused() {
        // Parse-mutate-reserialize: string replaces are hostage to the
        // file's formatting (the previous one-line anchors matched nothing,
        // so the corruption silently never applied).
        let mut document: serde_json::Value = serde_json::from_str(EMBEDDED_IR).expect("json");
        for record in document["commands"].as_array_mut().expect("commands").iter_mut() {
            if record["path"] == serde_json::json!(["ci"]) {
                record["legacy"] = serde_json::json!(["ygg-panic.py"]);
            }
        }
        let broken = serde_json::to_string(&document).expect("serialize");
        assert!(matches!(
            parse_and_validate(&broken),
            Err(LoadError::Refused(message)) if message.contains("not owned by its component")
        ));
    }

    #[test]
    fn unknown_role_is_refused() {
        let mut document: serde_json::Value = serde_json::from_str(EMBEDDED_IR).expect("json");
        for record in document["commands"].as_array_mut().expect("commands").iter_mut() {
            if record["path"] == serde_json::json!(["daemon"]) {
                record["role"] = serde_json::json!("spooky");
            }
        }
        let broken = serde_json::to_string(&document).expect("serialize");
        assert!(matches!(
            parse_and_validate(&broken),
            Err(LoadError::Refused(message)) if message.contains("unsupported command role")
        ));
    }

    #[test]
    fn wired_adapter_must_name_a_family() {
        let mut document: serde_json::Value = serde_json::from_str(EMBEDDED_IR).expect("IR");
        document["wired_adapters"]["not-a-family"] = serde_json::json!({"adapter": "example"});
        let broken = serde_json::to_string(&document).expect("serialize");
        assert!(matches!(
            parse_and_validate(&broken),
            Err(LoadError::Refused(message)) if message.contains("unknown family")
        ));
    }

    #[test]
    fn contract_hash_is_the_loaded_bytes_digest() {
        let loaded = load().expect("load");
        assert_eq!(loaded.contract_hash.len(), 64);
        let again = load().expect("load");
        assert_eq!(loaded.contract_hash, again.contract_hash);
    }
}
