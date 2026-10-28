//! Grammar over the shared command tree: parsing, help, completion and
//! local example search. No source read, process or network call exists on
//! any of these routes; that is the falsifier the tests enforce.

use crate::ir::{Loaded, Node, OptionEntry};
use std::collections::BTreeMap;

/// Parsed argument values, kept close to the argparse namespace shape the
/// Python front end produces (store -> Null default, store_true -> false,
/// append -> empty list) so identical invocations yield identical staged
/// request fingerprints.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValue {
    Str(String),
    Int(i64),
    Bool(bool),
    Null,
    List(Vec<String>),
}

impl ArgValue {
    pub fn python_json(&self) -> String {
        match self {
            ArgValue::Str(s) => serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()),
            ArgValue::Int(i) => i.to_string(),
            ArgValue::Bool(b) => b.to_string(),
            ArgValue::Null => "null".into(),
            ArgValue::List(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
        }
    }
}

pub type Args = BTreeMap<String, ArgValue>;

#[derive(Debug)]
pub enum ParseOutcome {
    Parsed {
        path: Vec<String>,
        args: Args,
        rest: Vec<String>,
    },
    /// A declared planned family: local help works, effects refuse with 3.
    PlannedFamily {
        path: Vec<String>,
        message: String,
    },
    /// Usage/argument error: exit 2.
    Invalid(String),
}

pub fn dest(option: &OptionEntry) -> String {
    option
        .flags
        .first()
        .map(|f| f.trim_start_matches('-').replace('-', "_"))
        .unwrap_or_default()
}

pub fn children<'a>(loaded: &'a Loaded, path: &[String]) -> Vec<&'a str> {
    let mut kids: Vec<&str> = loaded
        .tree
        .keys()
        .filter(|p| p.len() == path.len() + 1 && p.starts_with(path) && !loaded.tree[*p].record.hidden)
        .map(|p| p.last().unwrap().as_str())
        .collect();
    kids.sort_unstable();
    kids.dedup();
    kids
}

/// Walk the leading command words; stop at the first option-like token or
/// a word that is not a child of the current node.
pub fn context<'a>(loaded: &'a Loaded, words: &[String]) -> Vec<String> {
    let mut path: Vec<String> = Vec::new();
    for word in words {
        if word.starts_with('-') || !loaded.tree.contains_key(&join(&path, word)) {
            break;
        }
        path.push(word.clone());
    }
    path
}

fn join(path: &[String], word: &str) -> Vec<String> {
    let mut next = path.to_vec();
    next.push(word.to_string());
    next
}

fn node<'a>(loaded: &'a Loaded, path: &[String]) -> &'a Node {
    &loaded.tree[&path.to_vec()]
}

fn default_value(option: &OptionEntry) -> ArgValue {
    match option.action.as_str() {
        "store_true" | "store_false" => ArgValue::Bool(false),
        "append" => ArgValue::List(Vec::new()),
        _ => match &option.default {
            Some(serde_json::Value::String(s)) => ArgValue::Str(s.clone()),
            Some(serde_json::Value::Number(n)) => ArgValue::Int(n.as_i64().unwrap_or(0)),
            Some(serde_json::Value::Bool(b)) => ArgValue::Bool(*b),
            _ => ArgValue::Null,
        },
    }
}

fn initial_args(node: &Node) -> Args {
    let mut args = Args::new();
    for option in &node.options {
        args.insert(dest(option), default_value(option));
    }
    let record = &node.record;
    let operands: Vec<String> = if !record.operands.is_empty() {
        record.operands.clone()
    } else if !record.operand.is_empty() {
        vec![record.operand.clone()]
    } else {
        Vec::new()
    };
    for name in operands {
        args.insert(name, ArgValue::Null);
    }
    if record.passthrough {
        args.insert("harness_args".into(), ArgValue::Null);
    }
    args
}

/// Parse a leaf invocation. Validation lives in the contract (choices,
/// int_range, require_one_of, conflicts, operand_choices, required_options);
/// no front end keeps its own table.
pub fn parse(loaded: &Loaded, words: &[String]) -> ParseOutcome {
    let path = context(loaded, words);
    if path.is_empty() {
        let word = words.first().map(|s| s.as_str()).unwrap_or("");
        let near = close_matches(word, &children(loaded, &[]));
        let hint = if near.is_empty() {
            "; try ygg help".to_string()
        } else {
            format!("; try: {}", near.join(", "))
        };
        return ParseOutcome::Invalid(format!("unknown command {word:?}{hint}"));
    }
    let parsed = node(loaded, &path);
    let rest: Vec<String> = words[path.len()..].to_vec();
    if parsed.record.role == "planned" {
        let title = format!("ygg {}", path.join(" "));
        return ParseOutcome::PlannedFamily {
            path,
            message: format!(
                "{title}: planned family, not implemented in this generation; local help only ({title} --help)"
            ),
        };
    }
    if parsed.record.handler == "group" {
        let kids = children(loaded, &path).join(", ");
        return ParseOutcome::Invalid(format!(
            "unknown next word/options for ygg {}; choose: {}",
            path.join(" "),
            kids
        ));
    }
    if parsed.record.handler == "legacy" && !parsed.record.strict {
        return ParseOutcome::Parsed {
            path,
            args: Args::new(),
            rest,
        };
    }

    let usage = parser_usage(loaded, &path);
    let mut args = initial_args(parsed);
    let record = &parsed.record;
    let operand_names: Vec<String> = if !record.operands.is_empty() {
        record.operands.clone()
    } else if !record.operand.is_empty() {
        vec![record.operand.clone()]
    } else {
        Vec::new()
    };
    let mut positional: Vec<String> = Vec::new();
    let mut iter = rest.iter().peekable();
    let mut literal_tail = false;
    while let Some(word) = iter.next() {
        if literal_tail {
            positional.push(word.clone());
            continue;
        }
        if word == "--" {
            literal_tail = true;
            continue;
        }
        if word.len() > 1 && word.starts_with('-') {
            let (flag, inline_value) = match word.split_once('=') {
                Some((f, v)) => (f.to_string(), Some(v.to_string())),
                None => (word.clone(), None),
            };
            let option = parsed.options.iter().find(|o| o.flags.iter().any(|f| *f == flag));
            let Some(option) = option else {
                return ParseOutcome::Invalid(format!("unrecognized option: {flag}\n{usage}"));
            };
            let name = dest(option);
            match option.action.as_str() {
                "store_true" | "store_false" => {
                    args.insert(name, ArgValue::Bool(option.action == "store_true"));
                }
                "append" => {
                    let Some(value) = inline_value.or_else(|| iter.next().cloned()) else {
                        return ParseOutcome::Invalid(format!(
                            "argument {flag}: expected one argument\n{usage}"
                        ));
                    };
                    if !option.choices.is_empty() && !option.choices.contains(&value) {
                        return ParseOutcome::Invalid(format!(
                            "argument {flag}: invalid choice: {value:?} (choose from {})\n{usage}",
                            option.choices.join(", ")
                        ));
                    }
                    if let ArgValue::List(list) = args.get(&name).cloned().unwrap_or(ArgValue::List(vec![])) {
                        let mut list = list;
                        list.push(value);
                        args.insert(name, ArgValue::List(list));
                    }
                }
                _ => {
                    let Some(value) = inline_value.or_else(|| iter.next().cloned()) else {
                        return ParseOutcome::Invalid(format!(
                            "argument {flag}: expected one argument\n{usage}"
                        ));
                    };
                    if !option.choices.is_empty() && !option.choices.contains(&value) {
                        return ParseOutcome::Invalid(format!(
                            "argument {flag}: invalid choice: {value:?} (choose from {})\n{usage}",
                            option.choices.join(", ")
                        ));
                    }
                    args.insert(name, ArgValue::Str(value));
                }
            }
            continue;
        }
        positional.push(word.clone());
    }

    if record.passthrough {
        args.insert("harness_args".into(), ArgValue::List(positional));
        return finish(loaded, &path, parsed, args);
    }
    let mut slots = positional.into_iter();
    for name in &operand_names {
        match slots.next() {
            Some(value) => {
                args.insert(name.clone(), ArgValue::Str(value));
            }
            None => break,
        }
    }
    if let Some(extra) = slots.next() {
        return ParseOutcome::Invalid(format!("unrecognized arguments: {extra}\n{usage}"));
    }
    for (index, name) in operand_names.iter().enumerate() {
        if args.get(name) != Some(&ArgValue::Null) {
            continue;
        }
        let optional = record.optional_operand && operands_is_single(&operand_names, index);
        if !optional {
            return ParseOutcome::Invalid(format!(
                "the following arguments are required: {name}\n{usage}"
            ));
        }
    }
    finish(loaded, &path, parsed, args)
}

fn operands_is_single(operand_names: &[String], index: usize) -> bool {
    operand_names.len() == 1 && index == 0
}

fn finish(
    loaded: &Loaded,
    path: &[String],
    parsed: &Node,
    mut args: Args,
) -> ParseOutcome {
    let usage = parser_usage(loaded, path);
    let record = &parsed.record;
    for option in &parsed.options {
        if option.int_range.len() == 2 {
            let name = dest(option);
            if let Some(ArgValue::Str(raw)) = args.get(&name).cloned() {
                match raw.parse::<i64>() {
                    Ok(value) if value >= option.int_range[0] && value <= option.int_range[1] => {
                        args.insert(name, ArgValue::Int(value));
                    }
                    _ => {
                        return ParseOutcome::Invalid(format!(
                            "{} must be an integer from {} to {}\n{usage}",
                            option.flags.first().cloned().unwrap_or_default(),
                            option.int_range[0],
                            option.int_range[1]
                        ));
                    }
                }
            }
        }
    }
    if !record.require_one_of.is_empty() {
        let any = record
            .require_one_of
            .iter()
            .any(|name| !matches!(args.get(name), None | Some(ArgValue::Null) | Some(ArgValue::Bool(false))));
        if !any {
            let names: Vec<String> = record
                .require_one_of
                .iter()
                .map(|n| format!("--{n}"))
                .collect();
            return ParseOutcome::Invalid(format!("provide {}\n{usage}", names.join(" or ")));
        }
    }
    for group in &record.conflicts {
        let chosen = group
            .iter()
            .filter(|name| !matches!(args.get(*name), None | Some(ArgValue::Null) | Some(ArgValue::Bool(false))))
            .count();
        if chosen > 1 {
            let names: Vec<String> = group.iter().map(|n| format!("--{n}")).collect();
            return ParseOutcome::Invalid(format!(
                "choose either {}, not both\n{usage}",
                names.join(" or ")
            ));
        }
    }
    if !record.operand_choices.is_empty() && !record.operand.is_empty() {
        if let Some(ArgValue::Str(value)) = args.get(&record.operand).cloned() {
            if !record.operand_choices.contains(&value) {
                return ParseOutcome::Invalid(format!(
                    "supported {}: {}\n{usage}",
                    record.operand,
                    record.operand_choices.join(", ")
                ));
            }
        }
    }
    for name in &record.required_options {
        let present = match args.get(name) {
            Some(ArgValue::Str(v)) => !v.is_empty(),
            Some(ArgValue::List(v)) => !v.is_empty(),
            _ => false,
        };
        if !present {
            return ParseOutcome::Invalid(format!(
                "--{} is required\n{usage}",
                name.replace('_', "-")
            ));
        }
    }
    ParseOutcome::Parsed {
        path: path.to_vec(),
        args,
        rest: Vec::new(),
    }
}

fn close_matches(word: &str, candidates: &[&str]) -> Vec<String> {
    if word.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &str)> = candidates
        .iter()
        .filter_map(|c| {
            let d = edit_distance(word, c);
            (d <= 2 || c.starts_with(word)).then(|| (d, *c))
        })
        .collect();
    scored.sort();
    scored.truncate(3);
    scored.into_iter().map(|(_, c)| c.to_string()).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push(std::cmp::min(std::cmp::min(cur[j] + 1, prev[j + 1] + 1), prev[j] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// A readable usage line for error/help output. Text presentation is the
/// front end's own; grammar data comes only from the contract.
pub fn parser_usage(loaded: &Loaded, path: &[String]) -> String {
    let parsed = node(loaded, path);
    let title = format!("ygg {}", path.join(" "));
    let mut parts = vec![title];
    for option in &parsed.options {
        match option.action.as_str() {
            "store_true" | "store_false" => parts.push(format!("[{}]", option.flags.join(" "))),
            _ => parts.push(format!(
                "[{} {}]",
                option.flags.join(" "),
                dest(option).to_uppercase()
            )),
        }
    }
    let record = &parsed.record;
    let operands: Vec<String> = if !record.operands.is_empty() {
        record.operands.clone()
    } else if !record.operand.is_empty() {
        vec![record.operand.clone()]
    } else {
        Vec::new()
    };
    for (index, name) in operands.iter().enumerate() {
        if record.optional_operand && operands.len() == 1 {
            parts.push(format!("[{name}]"));
        } else if index == 0 || !record.optional_operand {
            parts.push(name.clone());
        } else {
            parts.push(format!("[{name}]"));
        }
    }
    if record.passthrough {
        parts.push("[owned-tool arguments ...]".into());
    }
    format!("usage: {}", parts.join(" "))
}

/// Help text for any node. Group and non-strict nodes teach the next word;
/// strict leaves show their own usage.
pub fn help_text(loaded: &Loaded, path: &[String]) -> String {
    let parsed = node(loaded, path);
    let kids = children(loaded, path);
    let title = format!("ygg{}", if path.is_empty() { String::new() } else { format!(" {}", path.join(" ")) });
    if kids.is_empty() && parsed.record.strict {
        let mut lines = vec![parser_usage(loaded, path), "".into()];
        if !parsed.record.summary.is_empty() {
            lines.push(parsed.record.summary.clone());
            lines.push("".into());
        }
        if !parsed.options.is_empty() {
            lines.push("options:".into());
            for option in &parsed.options {
                let flags = option.flags.join(", ");
                let arg = if matches!(option.action.as_str(), "store_true" | "store_false") {
                    String::new()
                } else {
                    format!(" {}", dest(option).to_uppercase())
                };
                lines.push(format!("  {flags}{arg}  {}", option.help));
            }
            lines.push("".into());
        }
        lines.push(examples_text(loaded, path));
        return lines.join("\n");
    }
    let mut lines = vec![
        format!("{title} — {}", parsed.record.summary),
        "".into(),
        format!(
            "Usage: {title} {}",
            if !kids.is_empty() { "<next word> [options]" } else { "[owned-tool arguments]" }
        ),
        "".into(),
    ];
    if !kids.is_empty() {
        lines.push("Next words:".into());
        for name in kids {
            let kid = node(loaded, &join(path, name));
            let effect = if kid.record.effect == "read" || kid.record.effect.is_empty() {
                String::new()
            } else {
                format!(" [{}]", kid.record.effect)
            };
            lines.push(format!("  {name:<12}{}{effect}", kid.record.summary));
        }
    } else {
        lines.push("Compatibility route: arguments retain the owned tool's validation.".into());
    }
    if !parsed.options.is_empty() {
        lines.push("\nOptions:".into());
        for option in &parsed.options {
            lines.push(format!("  {}  {}", option.flags.join(", "), option.help));
        }
    }
    lines.push("".into());
    lines.push(examples_text(loaded, path));
    lines.push("".into());
    lines.join("\n")
}

pub fn examples_text(loaded: &Loaded, path: &[String]) -> String {
    let found = all_examples(loaded, path);
    if found.is_empty() {
        return "Next: ygg help examples --search <task words>".into();
    }
    let mut lines = vec!["Examples:".into()];
    for (_, e) in found.iter().take(8) {
        lines.push(format!("  {}:\n    {}", e.title, e.command));
    }
    lines.join("\n")
}

fn all_examples<'a>(
    loaded: &'a Loaded,
    path: &[String],
) -> Vec<(Vec<String>, &'a crate::ir::ExampleEntry)> {
    let mut found = Vec::new();
    for (p, n) in &loaded.tree {
        if p.starts_with(path) {
            for e in &n.record.examples {
                found.push((p.clone(), e));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

pub fn search_examples(loaded: &Loaded, phrase: &str) -> Vec<serde_json::Value> {
    let words: Vec<String> = phrase.to_lowercase().split_whitespace().map(String::from).collect();
    let mut out = Vec::new();
    for (p, e) in all_examples(loaded, &[]) {
        let hay = format!("{} {} {}", e.title, e.tags, e.command).to_lowercase();
        if words.iter().all(|w| hay.contains(w)) {
            out.push(serde_json::json!({
                "title": e.title,
                "command": e.command,
                "effect": loaded.tree[&p].record.effect,
            }));
        }
    }
    out
}

/// Completion is catalog-only; operands and IDs never trigger discovery.
pub fn complete(loaded: &Loaded, words: &[String]) -> Vec<String> {
    let prefix = words.last().map(|s| s.as_str()).unwrap_or("");
    let preceding: Vec<String> = words[..words.len().saturating_sub(1)].to_vec();
    let path = context(loaded, &preceding);
    if !preceding.is_empty() && path.is_empty() {
        return Vec::new();
    }
    let parsed = node(loaded, &path);
    if !preceding.is_empty() {
        if let Some(last) = preceding.last() {
            for option in &parsed.options {
                if option.flags.iter().any(|f| f == last)
                    && !matches!(option.action.as_str(), "store_true" | "store_false")
                {
                    return option
                        .choices
                        .iter()
                        .filter(|c| c.starts_with(prefix))
                        .cloned()
                        .collect();
                }
            }
        }
    }
    let mut candidates: Vec<String> = children(loaded, &path)
        .into_iter()
        .map(String::from)
        .collect();
    for option in &parsed.options {
        candidates.extend(option.flags.iter().cloned());
    }
    candidates.push("--help".into());
    let mut out: Vec<String> = candidates
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|c| c.starts_with(prefix))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::load;
    use std::env;

    fn loaded() -> crate::ir::Loaded {
        // Unit tests always read the embedded contract; the env pointer is
        // cleared so an ambient YGG_COMMAND_IR cannot skew the fixture.
        // SAFETY: process env is only mutated here, to the same absent
        // state every loader path already treats as the default; no other
        // test in this target writes YGG_COMMAND_IR concurrently.
        unsafe { env::remove_var("YGG_COMMAND_IR") };
        load().expect("embedded contract loads")
    }

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_routes_parse_at_every_prefix() {
        let loaded = loaded();
        for path in loaded.tree.keys() {
            if path.is_empty() {
                continue; // root --help is intercepted in main before parse
            }
            let mut invocation = path.clone();
            invocation.push("--help".into());
            let outcome = parse(&loaded, &invocation);
            // --help is intercepted before parse in main; parse itself must
            // at worst reject it as arguments, never find an unknown command.
            if let ParseOutcome::Invalid(message) = outcome {
                assert!(
                    !message.contains("unknown command"),
                    "{path:?}: {message}"
                );
            }
        }
    }

    #[test]
    fn children_are_sorted_and_context_walks() {
        let loaded = loaded();
        let kids = children(&loaded, &[]);
        assert!(kids.windows(2).all(|w| w[0] <= w[1]));
        assert!(kids.contains(&"dream"));
        assert_eq!(context(&loaded, &words(&["dream", "ls", "--json"])), words(&["dream", "ls"]));
        assert_eq!(context(&loaded, &words(&["--json", "dream"])), Vec::<String>::new());
    }

    #[test]
    fn declarative_validations_reject_and_accept() {
        let loaded = loaded();
        for case in [
            vec!["service", "ps", "--limit", "0"],
            vec!["service", "ps", "--limit", "bad"],
            vec!["dream", "ls", "--state", "made-up"],
            vec!["dream", "inspect"],
            vec!["dream", "open"],
            vec!["dream", "open", "dream-42", "--pick"],
            vec!["dream", "topup", "--typo"],
            vec!["swarm", "pressure", "--lim", "2"],
            vec!["payload", "install"],
            vec!["completion", "zsh"],
        ] {
            let invocation = words(&case);
            assert!(
                matches!(parse(&loaded, &invocation), ParseOutcome::Invalid(_)),
                "expected rejection: {case:?}"
            );
        }
        for case in [
            vec!["dream", "open", "dream-42"],
            vec!["dream", "open", "--pick"],
            vec!["service", "ps", "--limit", "7"],
            vec!["completion", "bash"],
            vec!["payload", "install", "--dir", "somewhere"],
        ] {
            let invocation = words(&case);
            assert!(
                matches!(parse(&loaded, &invocation), ParseOutcome::Parsed { .. }),
                "expected acceptance: {case:?}"
            );
        }
    }

    #[test]
    fn limit_is_coerced_to_int_for_the_staged_fingerprint() {
        let loaded = loaded();
        match parse(&loaded, &words(&["dream", "ls"])) {
            ParseOutcome::Parsed { args, .. } => {
                assert_eq!(args.get("limit"), Some(&ArgValue::Int(50)));
                assert_eq!(args.get("json"), Some(&ArgValue::Bool(false)));
            }
            other => panic!("expected parsed, got {other:?}"),
        }
    }

    #[test]
    fn non_strict_legacy_passthrough_keeps_every_token() {
        let loaded = loaded();
        match parse(&loaded, &words(&["memory", "sync", "--harness", "example", "--ns=-area"])) {
            ParseOutcome::Parsed { rest, args, .. } => {
                assert_eq!(rest, words(&["sync", "--harness", "example", "--ns=-area"]));
                assert!(args.is_empty());
            }
            other => panic!("expected passthrough, got {other:?}"),
        }
    }

    #[test]
    fn planned_families_refuse_effects_but_keep_help() {
        let loaded = loaded();
        match parse(&loaded, &words(&["daemon", "status"])) {
            ParseOutcome::PlannedFamily { message, .. } => {
                assert!(message.contains("planned family"));
            }
            other => panic!("expected planned refusal, got {other:?}"),
        }
        assert!(help_text(&loaded, &words(&["daemon"])).contains("planned"));
    }

    #[test]
    fn completion_is_catalog_only() {
        let loaded = loaded();
        let out = complete(&loaded, &words(&["dream", "ls", "--state", "f"]));
        assert_eq!(out, vec!["failed".to_string()]);
        let out = complete(&loaded, &words(&["drea"]));
        assert!(out.contains(&"dream".to_string()));
        let out = complete(&loaded, &words(&["dream", "logs", "some-id", ""]));
        // The operand word itself is never a completion candidate.
        assert!(!out.iter().any(|c| c == "some-id"));
        assert!(out.contains(&"--attempt".to_string()));
    }

    #[test]
    fn examples_search_is_local_and_tagged() {
        let loaded = loaded();
        let found = search_examples(&loaded, "unused quota");
        assert!(!found.is_empty());
        for e in &found {
            assert!(e["command"].as_str().unwrap().starts_with("ygg"));
        }
    }

    #[test]
    fn unknown_command_suggests_close_matches() {
        let loaded = loaded();
        match parse(&loaded, &words(&["drem"])) {
            ParseOutcome::Invalid(message) => assert!(message.contains("dream"), "{message}"),
            other => panic!("expected invalid, got {other:?}"),
        }
    }
}
