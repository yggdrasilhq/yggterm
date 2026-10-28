#!/usr/bin/env python3
"""Host porcelain for the shared deterministic swarm command contract."""
import hashlib
import json
import os
from pathlib import Path
import sys

try:
    from ygg_command_tree import TREE, CONTRACT_HASH, complete, context, help_text, parse, search_examples
except (ImportError, ValueError, TypeError, KeyError, OSError) as exc:
    print("ygg: command catalog unavailable (" + type(exc).__name__ + "); install a matching generation", file=sys.stderr)
    raise SystemExit(3)


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] == "--version":
        argv[0] = "version"
    # Respect the literal-arguments delimiter. No imported host adapter or
    # subprocess is touched on ANY help route, including legacy leaves.
    before_delimiter = argv[:argv.index("--")] if "--" in argv else argv
    if not argv or any(x in ("-h", "--help") for x in before_delimiter):
        print(help_text(context(before_delimiter)))
        return 0
    if argv[0] == "help" and (len(argv) == 1 or argv[1] != "examples"):
        path = context(argv[1:])
        if argv[1:] and not path:
            print("ygg: unknown help topic", file=sys.stderr)
            return 2
        print(help_text(path))
        return 0
    if tuple(argv) in TREE and TREE[tuple(argv)].handler == "group":
        print(help_text(tuple(argv)))
        return 0
    if argv[0] == "complete":
        words = argv[1:]
        if words[:1] == ["--"]:
            words = words[1:]
        print("\n".join(complete(words)))
        return 0
    # Preserve the old row-board flag form while noun-only board now teaches
    # the topic-board verbs. Its existing argparse validator remains owner.
    if argv[0] == "board" and len(argv) > 1 and argv[1].startswith("-"):
        argv.insert(1, "rows")
    expected_contract = os.environ.get("YGG_COMMAND_CONTRACT_HASH")
    if expected_contract and expected_contract != CONTRACT_HASH:
        print("ygg: native/component command contracts differ; install a matching generation", file=sys.stderr)
        return 3
    try:
        path, args, rest = parse(argv)
    except ValueError as exc:
        print("ygg: " + str(exc), file=sys.stderr)
        return 2
    node = TREE[path]
    if node.handler == "version":
        print("ygg command contract 1 (swarm introspection)")
        return 0
    if node.handler == "examples":
        found = search_examples(args.search)
        if args.json:
            print(json.dumps({"schema_version": 1, "examples": found}, indent=2))
        else:
            for e in found:
                print(e["title"] + " [" + e["effect"] + "]\n  " + e["command"])
            if not found:
                print("No matching local example; try fewer words.")
        return 0
    if node.handler == "completion":
        print('''_ygg_complete() {
  mapfile -t COMPREPLY < <(ygg complete -- "${COMP_WORDS[@]:1:$COMP_CWORD}")
}
complete -F _ygg_complete ygg''')
        return 0
    if node.handler == "legacy":
        target, *fixed = node.legacy
        if target == "@msgboard":
            os.execvp("msgboard", ["msgboard", *fixed, *rest])
        root = Path(os.environ.get("YGG_BIN_DIR", Path(__file__).resolve().parent))
        # Dispersal carries memory copies for bootstrap, but its owner-managed
        # runner is authoritative. A newer owner update must not be bypassed
        # by a stale copy of the same filename in the payload directory.
        if target in ("ygg-memory", "ygg-memory.py"):
            executable = Path(os.environ.get("YGG_MEMORY_BIN", Path.home() / ".local/bin" / target))
        else:
            executable = root / target
        if not executable.is_file():
            print("ygg: compatibility tool unavailable: " + target, file=sys.stderr)
            return 3
        os.execv(str(executable), [str(executable), *fixed, *rest])
    # This is an honest staged availability result, never a healthy empty
    # ledger. Source adapters are admitted separately from local help.
    request = {"command": list(path), "arguments": vars(args)}
    snapshot_id = "unavailable-" + hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest()[:16]
    result = {"schema_version": 1, "snapshot_id": snapshot_id, "as_of": None,
              "config_hash": None, "coverage": [{"source": ".".join(path),
                  "quality": "unavailable", "reason": "adapter_not_implemented"}],
              "items": [], "next_cursor": None,
              "findings": [{"code": "ADAPTER_NOT_IMPLEMENTED", "command": list(path)}]}
    if getattr(args, "json", False):
        print(json.dumps(result, indent=2))
    else:
        print("ygg " + " ".join(path) + ": source adapter not implemented; coverage UNKNOWN.")
        print("Help and examples are available; an empty result would not prove zero work.")
    return 3


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except OSError as exc:
        print("ygg: owned component unavailable (" + type(exc).__name__ + ")", file=sys.stderr)
        raise SystemExit(3)
