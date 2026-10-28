"""One command tree for parsing, help, local examples and completion.

This module performs no source reads or process/network calls. The host
porcelain resolves effects only after parsing. Opaque tools retain owned
validators; the older ad-hoc dream/fleet tools get an explicit supported
argument contract. Help never invokes either kind.
"""
import argparse
import difflib
from dataclasses import dataclass, replace


@dataclass(frozen=True)
class Option:
    flags: tuple
    help: str
    action: str = "store"
    choices: tuple = ()
    default: object = None


@dataclass(frozen=True)
class Example:
    title: str
    command: str
    tags: str = ""


@dataclass(frozen=True)
class Command:
    summary: str
    effect: str = "read"
    handler: str = "group"
    options: tuple = ()
    operand: str = ""
    optional_operand: bool = False
    examples: tuple = ()
    legacy: tuple = ()
    # Strict compatibility leaves (such as topup) must not ignore typos.
    strict: bool = True
    hidden: bool = False


def opt(flag, help, action="store", choices=(), default=None):
    return Option((flag,), help, action, tuple(choices), default)


JSON = opt("--json", "Emit a versioned machine-readable result.", "store_true")
COMMON = (JSON, opt("--host", "Host name or all (configured enrollment only)."),
          opt("--since", "Relative age, such as 7d, or an absolute timestamp."),
          opt("--until", "Absolute timestamp or future duration, such as +7d."),
          opt("--limit", "Maximum result rows.", default="50"),
          opt("--cursor", "Continue a page of the same snapshot."),
          opt("--snapshot", "Read an immutable retained snapshot ID."))
STATES = ("planned", "scheduled", "queued", "running", "failed", "completed",
          "cancelled", "interrupted", "unknown", "retry_wait", "cooldown_wait")
FILTER = (opt("--state", "Match attempt states; repeats are OR.", "append", STATES),
          opt("--review", "Composer disposition.", choices=("unreviewed", "investigating",
              "repair-proposed", "repair-running", "awaiting-verification", "verified",
              "false-positive", "deferred")))


TREE = {
    (): Command("Inspect and operate the swarm. Type a noun to learn the next word."),
    ("dream",): Command("Inspect autonomous work, attempts and their evidence."),
    ("dream", "ps"): Command("Show active dream attempts.", handler="query", options=COMMON),
    ("dream", "ls"): Command("Browse planned, current and historical dreams.", handler="query",
        options=COMMON + FILTER + (opt("--all", "Include retained attempt history.", "store_true"),),
        examples=(Example("Find failed dreams not examined by the composer",
            "ygg dream ls --all --state failed --review unreviewed --since 7d", "failures history composer"),)),
    ("dream", "inspect"): Command("Explain a work item, its retries and repair history.", handler="query",
        operand="id", options=COMMON + (opt("--attempts", "Include every retained attempt.", "store_true"),
            opt("--repairs", "Include linked composer notes and repair outcomes.", "store_true")),
        examples=(Example("Trace a failed work item through retries and repairs",
            "ygg dream inspect dream-42 --attempts --repairs", "history evidence"),)),
    ("dream", "logs"): Command("Read evidence for an exact dream attempt.", handler="query",
        operand="id", options=COMMON + (opt("--attempt", "Exact attempt ID."),),
        examples=(Example("Read the failed attempt, not a later retry",
            "ygg dream logs dream-42 --attempt attempt-3", "failure logs"),)),
    ("dream", "open"): Command("Select and inspect a live session or read-only transcript.",
        handler="open", effect="attach", operand="id", optional_operand=True,
        options=COMMON + FILTER + (opt("--pick", "Choose interactively; never resumes work.", "store_true"),
                                  opt("--attempt", "Exact attempt ID.")),
        examples=(Example("Select a failed attempt for human inspection",
            "ygg dream open --pick --state failed", "picker session harness"),)),
    ("dream", "status"): Command("Existing orchestrator status (compatibility).", handler="legacy",
        options=(JSON,), legacy=("ygg-dream-orchestrator.py", "status"),
        examples=(Example("Read the existing spawner ledger", "ygg dream status --json"),)),
    ("dream", "topup"): Command("Stage work through existing permission and arming gates.",
        handler="legacy", effect="dispatch", options=(JSON,), legacy=("ygg-dream-orchestrator.py", "topup")),
    ("dream", "supervise"): Command("Run the existing gated supervision pass.",
        handler="legacy", effect="change", options=(JSON,), legacy=("ygg-dream-orchestrator.py", "supervise")),
    ("service",): Command("Locate expected and observed swarm services and resources."),
    ("service", "ps"): Command("Compare expected services with observed instances.", handler="query",
        options=COMMON, examples=(Example("Find where each service runs", "ygg service ps --host all", "placement missing duplicate"),)),
    ("service", "inspect"): Command("Inspect one service's identity and health.", handler="query",
        operand="id", options=COMMON,
        examples=(Example("Inspect a service on its owning host", "ygg service inspect service-12 --host worker-a"),)),
    ("service", "top"): Command("Show compute ownership without counting shared resources twice.",
        handler="query", options=COMMON + (opt("--by", "Group resource ownership.", choices=("dream", "service", "host"), default="dream"),),
        examples=(Example("Compare dreamer compute", "ygg service top --by dream", "cpu memory resources"),)),
    ("service", "logs"): Command("Read bounded service diagnostics.", handler="query", operand="id", options=COMMON,
        examples=(Example("Read an hour of service failures", "ygg service logs service-12 --host worker-a --since 1h"),)),
    ("schedule",): Command("Read periodic jobs and revisable conditional commitments."),
    ("schedule", "ls"): Command("List planned work with its conditions and next checks.", handler="query",
        options=COMMON + (opt("--due", "Due boundary, such as before:month-end."),),
        examples=(Example("Find work to evaluate before month end", "ygg schedule ls --due before:month-end", "monthly conditional plans"),)),
    ("schedule", "inspect"): Command("Read a commitment's source, conditions and authority.", handler="query", operand="id", options=COMMON,
        examples=(Example("Inspect the transcription priority conditions", "ygg schedule inspect month-end-transcription"),)),
    ("schedule", "why"): Command("Explain why planned work is due, held or awaiting evaluation.", handler="query", operand="id", options=COMMON,
        examples=(Example("Explain a deferred monthly sweep", "ygg schedule why month-end-transcription", "blocked prerequisites"),)),
    ("schedule", "history"): Command("Read previous decisions and reasons for changing them.", handler="query", operand="id", options=COMMON,
        examples=(Example("See how a future commitment changed", "ygg schedule history month-end-transcription", "opinions revisions"),)),
    ("swarm",): Command("Scan heartbeat, capacity balance and operational evidence."),
    ("swarm", "status"): Command("Summarize state and observation coverage.", handler="query", options=COMMON,
        examples=(Example("Start a system inspection", "ygg swarm status", "overview health"),)),
    ("swarm", "heartbeat"): Command("Separate service pulse, observer freshness and useful progress.", handler="query",
        options=COMMON + (opt("--layers", "Show each progress layer.", "store_true"),),
        examples=(Example("Find a live service doing no useful work", "ygg swarm heartbeat --layers", "stalled pulse"),)),
    ("swarm", "pressure"): Command("Explain usable funding, eligible work and execution bottlenecks.", handler="query",
        options=COMMON + (opt("--by", "Group capacity by native funding or execution scope.", choices=("plan", "host", "plane"), default="plan"),
                          opt("--explain", "Include constraints and unknown inputs.", "store_true")),
        examples=(Example("Explain unused quota and idle capacity", "ygg swarm pressure --by plan --explain", "unused quota capacity idle funding balance"),)),
    ("swarm", "events"): Command("Read retained operational events and repair dispositions.", handler="query",
        options=COMMON + (opt("--subsystem", "Subsystem name.", "append"),
                          opt("--severity", "Minimum event severity.", choices=("debug", "info", "warning", "error", "critical"))),
        examples=(Example("Inspect yesterday's memory warnings", "ygg swarm events --subsystem memory --severity warning --since 1d", "maintenance journal failures"),)),
    ("swarm", "check"): Command("Falsify named health assertions; unknown evidence never passes.", handler="query",
        options=COMMON + (opt("--scope", "Assertion family.", choices=("all", "dispatch", "services", "funding", "memory"), default="all"),),
        examples=(Example("Check whether dispatch is working", "ygg swarm check --scope dispatch", "assertions falsify"),)),
    ("help",): Command("Read local command help and search complete examples."),
    ("help", "examples"): Command("Find a complete command by the task you want to perform.", handler="examples",
        options=(JSON, opt("--search", "Words in the example's task, tags or command.")),
        examples=(Example("Find commands about spare funding", 'ygg help examples --search "unused quota"'),)),
    ("completion",): Command("Print shell completion setup (local; no fleet probes).", handler="completion",
        operand="shell", examples=(Example("Install Bash completion in the current shell", "ygg completion bash"),)),
    ("complete",): Command("Return local next-word candidates for shell completion.", handler="complete"),
    ("version",): Command("Print the command contract version.", handler="version"),
    ("agents",): Command("Existing agent registry compatibility view."),
    ("agents", "ps"): Command("Read the existing agent registry; coverage is backend-defined.",
        handler="legacy", options=(JSON,), legacy=("ygg-fleet.py", "agents", "ps")),
    ("fleet",): Command("Existing fleet compatibility views."),
    ("fleet", "pressure"): Command("Read the existing fleet pressure view.", handler="legacy",
        options=(JSON,), legacy=("ygg-fleet.py", "fleet", "pressure")),
    ("board",): Command("Read topic boards, or access the existing row-board view."),
    ("board", "scan"): Command("Find unmaterialized feature dreams.", handler="legacy",
        options=(JSON,), legacy=("ygg-fleet.py", "board", "scan")),
}

# These validators already live in the owned tools. Do not replace their
# argument contracts with guessed partial copies. Help is always local.
for name, target, summary, example, effect in (
    ("memory", "ygg-memory", "Cross-harness semantic memory.", "ygg memory status --harness codex --ns=-home-user-project --json", "legacy"),
    ("auth", "ygg-auth.py", "Existing local account-policy tools.", "ygg auth status", "legacy"),
    ("ci", "ygg-ci.py", "Single integration build plane.", "ygg ci status --json", "legacy"),
    ("init", "ygg-init.sh", "Deterministic onboarding.", "ygg init --tier 0", "legacy"),
    ("doctor", "ygg-init.sh", "Existing deterministic node diagnostics.", "ygg doctor", "legacy"),
):
    TREE[(name,)] = Command(summary, handler="legacy", effect=effect,
        legacy=(target, name) if name in ("init", "doctor") else (target,),
        strict=False, examples=(Example("Use the existing owned tool", example),))
for name in ("row", "bridge", "spawn", "deliver", "fold", "land", "claim", "panic", "booter", "monitor", "babysit"):
    target = {"row": "ygg_row.py", "bridge": "ygg_bridge.py", "claim": "ygg-claim.sh"}.get(name, "ygg-" + name + ".py")
    TREE[(name,)] = Command("Existing " + name + " compatibility tool.", effect="legacy",
        handler="legacy", legacy=(target,), strict=False)
TREE[("board", "rows")] = Command("Existing row-board view and row metadata operations.",
    effect="legacy", handler="legacy", legacy=("ygg-board.py",), strict=False)
for verb in ("read", "list", "summary", "post", "answer", "graduate"):
    TREE[("board", verb)] = Command("Topic-board " + verb + " through msgboard.",
        handler="legacy", effect="read" if verb in ("read", "list", "summary") else "change",
        legacy=("@msgboard", verb), strict=False)

# Preserve the old direct-script spelling only for known owned tools. Never
# resolve arbitrary paths or execute a merely similar filename.
for target in {n.legacy[0] for n in TREE.values() if n.legacy and not n.legacy[0].startswith("@")}:
    TREE[(target,)] = Command("Direct owned-tool compatibility spelling.", effect="legacy",
        handler="legacy", legacy=(target,), strict=False, hidden=True)

# The old ad-hoc tools silently ignored unknown arguments. Both their noun
# forms and their direct-script spellings now reject unsupported flags before
# effects; direct spellings must not bypass the canonical validator.
target = "ygg-dream-orchestrator.py"
TREE[(target,)] = Command("Compatibility spelling of ygg dream.", hidden=True)
for verb in ("status", "topup", "supervise"):
    TREE[(target, verb)] = replace(TREE[("dream", verb)], examples=())
target = "ygg-fleet.py"
TREE[(target,)] = Command("Compatibility fleet views.", hidden=True)
for verb, child, canonical in (("agents", "ps", ("agents", "ps")),
                                ("fleet", "pressure", ("fleet", "pressure")),
                                ("board", "scan", ("board", "scan"))):
    n = TREE[canonical]
    TREE[(target, verb)] = replace(n, legacy=(target, verb), examples=())
    TREE[(target, verb, child)] = replace(n, examples=())
TREE[(target, "pressure")] = replace(TREE[("fleet", "pressure")],
                                      legacy=(target, "pressure"), examples=())


def children(path):
    return sorted(p[-1] for p in TREE if len(p) == len(path) + 1 and p[:-1] == path and not TREE[p].hidden)


def context(words):
    path = ()
    for word in words:
        if word.startswith("-") or path + (word,) not in TREE:
            break
        path += (word,)
    return path


def parser_for(path):
    node = TREE[path]
    parser = argparse.ArgumentParser(prog="ygg" + (" " + " ".join(path) if path else ""),
        description=node.summary, allow_abbrev=False,
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=examples_text(path))
    for o in node.options:
        kw = {"help": o.help, "action": o.action}
        if o.choices:
            kw["choices"] = o.choices
        if o.default is not None:
            kw["default"] = o.default
        parser.add_argument(*o.flags, **kw)
    if node.operand:
        parser.add_argument(node.operand, **({"nargs": "?"} if node.optional_operand else {}))
    return parser


def examples(path=()):
    return [(p, e) for p, n in TREE.items() if p[:len(path)] == path for e in n.examples]


def examples_text(path):
    found = examples(path)
    if not found:
        return "Next: ygg help examples --search <task words>"
    return "Examples:\n" + "\n".join("  " + e.title + ":\n    " + e.command for _, e in found[:8])


def help_text(path=()):
    node = TREE[path]
    kids = children(path)
    title = "ygg" + (" " + " ".join(path) if path else "")
    if not kids and node.strict:
        return parser_for(path).format_help()
    lines = [title + " — " + node.summary, "", "Usage: " + title + (" <next word> [options]" if kids else " [owned-tool arguments]"), ""]
    if kids:
        lines.append("Next words:")
        for name in kids:
            n = TREE[path + (name,)]
            lines.append("  " + name.ljust(12) + n.summary + (" [" + n.effect + "]" if n.effect not in ("read",) else ""))
    else:
        lines.append("Compatibility route: arguments retain the owned tool's validation.")
    if node.options:
        lines.append("\nOptions:")
        lines.extend("  " + ", ".join(o.flags) + "  " + o.help for o in node.options)
    return "\n".join(lines) + "\n\n" + examples_text(path) + "\n"


def parse(words):
    """Return (path, namespace, raw leaf args); no effect may precede this."""
    path = context(words)
    if not path:
        word = words[0] if words else ""
        near = difflib.get_close_matches(word, children(()), n=3)
        raise ValueError("unknown command " + repr(word) + ("; try: " + ", ".join(near) if near else "; try ygg help"))
    node = TREE[path]
    rest = words[len(path):]
    if node.handler == "group":
        raise ValueError("unknown next word/options for ygg " + " ".join(path) + "; choose: " + ", ".join(children(path)))
    if node.handler == "legacy" and not node.strict:
        return path, argparse.Namespace(), rest
    args = parser_for(path).parse_args(rest)
    if path == ("dream", "open") and not (args.id or args.pick):
        parser_for(path).error("provide an exact ID or --pick")
    if path == ("dream", "open") and args.id and args.pick:
        parser_for(path).error("choose either an exact ID or --pick, not both")
    if hasattr(args, "limit"):
        try:
            args.limit = int(args.limit)
            if not 1 <= args.limit <= 1000:
                raise ValueError()
        except ValueError:
            parser_for(path).error("--limit must be an integer from 1 to 1000")
    if path == ("completion",) and args.shell != "bash":
        parser_for(path).error("supported shell: bash")
    return path, args, rest


def complete(words):
    """Completion is catalog-only; operands/IDs never trigger discovery."""
    prefix = words[-1] if words else ""
    preceding = words[:-1] if words else []
    path = context(preceding)
    if preceding and not path:
        return []
    node = TREE[path]
    if preceding:
        for o in node.options:
            if preceding[-1] in o.flags and o.action not in ("store_true", "store_false"):
                return [c for c in o.choices if c.startswith(prefix)]
    candidates = children(path) + [f for o in node.options for f in o.flags] + ["--help"]
    return sorted(c for c in set(candidates) if c.startswith(prefix))


def search_examples(phrase):
    words = phrase.lower().split() if phrase else []
    return [{"title": e.title, "command": e.command, "effect": TREE[p].effect}
            for p, e in examples() if all(w in (e.title + " " + e.tags + " " + e.command).lower() for w in words)]
