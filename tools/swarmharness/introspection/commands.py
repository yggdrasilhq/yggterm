"""One command tree for parsing, help, local examples and completion.

This module performs no source reads or process/network calls. The host
porcelain resolves effects only after parsing. Opaque tools retain owned
validators; the older ad-hoc dream/fleet tools get an explicit supported
argument contract. Help never invokes either kind.
"""
import argparse
import difflib
from dataclasses import dataclass


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
    component: str = ""
    operands: tuple = ()
    passthrough: bool = False


def _load_tree():
    # This is local command data, never a fleet/authority source read.
    import hashlib
    import json
    from pathlib import Path
    data_path = Path(__file__).resolve().with_name("commands.json")
    raw = data_path.read_bytes()
    document = json.loads(raw)
    if document.get("schema_version") != 1:
        raise ValueError("unsupported command contract schema")
    option_sets = document["option_sets"]
    definitions = {}
    for record in document["commands"]:
        path = tuple(record["path"])
        if path in definitions or any(not isinstance(x, str) for x in path):
            raise ValueError("invalid/duplicate command path")
        definitions[path] = record
    tree = {}
    def options(records, stack=()):
        expanded = []
        for entry in records:
            if "$ref" in entry:
                key = entry["$ref"]
                if key in stack or key not in option_sets:
                    raise ValueError("cyclic/unknown option set")
                expanded.extend(options(option_sets[key], stack + (key,)))
            else:
                fields = dict(entry)
                fields["flags"] = tuple(fields["flags"])
                fields["choices"] = tuple(fields.get("choices", ()))
                expanded.append(Option(**fields))
        return tuple(expanded)
    def resolve(path, stack=()):
        if path in tree:
            return tree[path]
        if path in stack or path not in definitions:
            raise ValueError("cyclic/unknown command alias")
        record = definitions[path]
        if "alias_of" in record:
            base = resolve(tuple(record["alias_of"]), stack + (path,))
            fields = dict(base.__dict__)
        else:
            fields = {}
        fields.update({k:v for k,v in record.items() if k not in ("path", "alias_of")})
        if "options" in record:
            fields["options"] = options(record["options"])
        if "examples" in record:
            fields["examples"] = tuple(Example(**e) for e in record["examples"])
        for key in ("legacy", "operands"):
            if key in record:
                fields[key] = tuple(record[key])
        tree[path] = Command(**fields)
        return tree[path]
    for path in definitions:
        resolve(path)
    if () not in tree or any(p and p[:-1] not in tree for p in tree):
        raise ValueError("command contract has missing parent")
    return tree, hashlib.sha256(raw).hexdigest()


TREE, CONTRACT_HASH = _load_tree()


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
    for name in node.operands or ((node.operand,) if node.operand else ()):
        parser.add_argument(name, **({"nargs": "?"} if node.optional_operand else {}))
    if node.passthrough:
        parser.add_argument("harness_args", nargs=argparse.REMAINDER)
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
