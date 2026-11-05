#!/usr/bin/env python3
"""ledger_cli.py — minimal local CLI for the working-ledger preview slice.

Local work-context owner; fleet execution and completion authority remain
with their canonical stores. Local help and examples
never mutate, create a store, or call a model. Pasted command text is
stored as data, never executed.

Surface: create / list / show / note / edit / history / resume / ops.
Output: human tables/details by default (payload rendered as concise
structured details, control chars escaped, bounded); `--json` emits a
versioned envelope with full values on stdout only; diagnostics go to
stderr. Flags are strict (no abbreviation). `--dry-run` plans a mutation
without any effect.
Exit codes: 0 ok (applied, or duplicate replay of an applied op),
2 usage/path, 3 store absent, 4 op-conflict hold, 5 corrupt/unknown
schema, 6 entry not found, 7 recorded but NOT applied — a stale CAS
proposal retained as a proposal, and the replay of such an op stays
not applied; never reported as applied/ok/exit 0.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import uuid as uuid_mod
from pathlib import Path

from working_ledger import (
    DEFAULT_DB_NAME, ENTRY_UUID_NS, MAX_EVENT_BYTES, MAX_NOTE_BYTES,
    MAX_RESUME_NOTES, ConflictHold,
    CorruptStore, EntryNotFound, Ledger, LedgerError, PathPolicy, SchemaUnknown,
    StoreAbsent, UsageError, actor_context, canonical_json, default_db_path,
    esc, known_roots, now_iso, parse_json_strict, sha256_hex, validate_db_path,
    validate_body, validate_event_budget,
)

ENV_VERSION = 1
EXIT = {"usage": 2, "path_rejected": 2, "store_absent": 3, "op_conflict_hold": 4,
        "store_corrupt": 5, "schema_unknown": 5, "entry_not_found": 6,
        "op_not_applied": 7}
NOT_APPLIED_CODE = "op_not_applied"
DEFAULT_LIST_LIMIT = 200


# ---------- output helpers ----------

def err_envelope(code, message):
    return {"v": ENV_VERSION, "ok": False,
            "error": {"code": code, "message": message}, "ledger": None}


def ok_envelope(ledger, data):
    return {"v": ENV_VERSION, "ok": True, "ledger": ledger, "data": data}


def h_table(headers, rows):
    cells = [[esc(c) for c in r] for r in rows]
    widths = [len(h) for h in headers]
    for r in cells:
        for i, c in enumerate(r):
            widths[i] = max(widths[i], len(c))
    out = ["  ".join(h.ljust(widths[i]) for i, h in enumerate(headers)),
           "  ".join("-" * w for w in widths)]
    out += ["  ".join(c.ljust(widths[i]) for i, c in enumerate(r)) for r in cells]
    return out


def h_kv(pairs):
    return [f"{esc(k)}: {esc(str(v))}" for k, v in pairs]


def payload_human(obj, depth=0) -> str:
    """Concise structured human rendering of a payload: one key per line,
    long values truncated, control chars escaped via ensure_ascii. Full
    values are available through --json (machine output)."""
    pad = "  " * (depth + 1)

    def one(v):
        s = json.dumps(v, ensure_ascii=True, sort_keys=True, allow_nan=False)
        return s if len(s) <= 120 else s[:117] + "…<%d chars>" % len(s)

    if isinstance(obj, dict) and obj:
        lines = []
        for k in sorted(obj, key=str):
            v = obj[k]
            if isinstance(v, (dict, list)) and v:
                lines.append("%s%s:" % (pad, esc(str(k))))
                lines.append(payload_human(v, depth + 1))
            else:
                lines.append("%s%s: %s" % (pad, esc(str(k)), one(v)))
        return "\n".join(lines)
    if isinstance(obj, list) and obj:
        shown = ["%s- %s" % (pad, one(v)) for v in obj[:20]]
        if len(obj) > 20:
            shown.append("%s… <%d more>" % (pad, len(obj) - 20))
        return "\n".join(shown)
    return "%s%s" % (pad, one(obj))


# ---------- input helpers ----------

def target_db(args):
    home = Path.home()
    db_arg = getattr(args, "db", None)
    db = Path(db_arg).expanduser() if db_arg else default_db_path(home)
    return validate_db_path(db, known_roots(home))


def build_actor(args):
    label = getattr(args, "session_label", None)
    source = None
    if label:
        source = "flag"
    elif os.environ.get("YGG_SESSION_LABEL"):
        label = os.environ["YGG_SESSION_LABEL"]
        source = "env"
    return actor_context(label, source)


def read_stream(stream, cap):
    data = stream.read(cap + 1)
    if len(data) > cap:
        raise UsageError(f"input exceeds {cap} byte cap")
    return data


def read_text(args, cap=MAX_NOTE_BYTES):
    text, file = getattr(args, "text", None), getattr(args, "file", None)
    if (text is None) == (file is None):
        raise UsageError("provide exactly one of --text or --file ('-' = stdin)")
    if file is not None:
        if file == "-":
            data = read_stream(sys.stdin.buffer, cap)
        else:
            try:
                with open(file, "rb") as f:
                    data = read_stream(f, cap)
            except OSError as e:
                raise UsageError(f"cannot read --file: {e}") from None
    else:
        data = text.encode("utf-8", "surrogateescape")
    if len(data) > cap:
        raise UsageError(f"input exceeds {cap} byte cap")
    try:
        s = data.decode("utf-8")
    except UnicodeDecodeError:
        raise UsageError("input is not valid UTF-8") from None
    if "\x00" in s:
        raise UsageError("NUL bytes are not allowed in notes")
    return s


def read_payload(args, required=False):
    inline, pfile = args.payload, args.payload_file
    if inline is not None and pfile is not None:
        raise UsageError("provide only one of --payload or --payload-file")
    if inline is None and pfile is None:
        if required:
            raise UsageError("provide --payload or --payload-file")
        return {}
    if pfile is not None:
        try:
            if pfile == "-":
                data = read_stream(sys.stdin.buffer, MAX_EVENT_BYTES)
            else:
                with open(pfile, "rb") as f:
                    data = read_stream(f, MAX_EVENT_BYTES)
            raw = data.decode("utf-8", "strict")
        except OSError as e:
            raise UsageError(f"cannot read --payload-file: {e}") from None
        except UnicodeDecodeError:
            raise UsageError("payload file is not valid UTF-8") from None
    else:
        raw = inline
    obj = parse_json_strict(raw, MAX_EVENT_BYTES)
    if not isinstance(obj, dict):
        raise UsageError("payload must be a JSON object")
    return obj


# ---------- mutation plumbing ----------

def run_mutation(args, kind, entry_uuid, body, needs_store=False, operation_uuid=None):
    db = target_db(args)
    actor = build_actor(args)
    canonical = canonical_json({"kind": kind, "entry_uuid": entry_uuid, "body": body})
    validate_body(kind, body)
    validate_event_budget(canonical, actor)
    if len(canonical.encode("utf-8")) > MAX_EVENT_BYTES:
        raise UsageError("event exceeds byte bound")
    ph = sha256_hex(canonical)
    op = operation_uuid or getattr(args, "op", None) or str(uuid_mod.uuid4())
    try:
        valid_op = str(uuid_mod.UUID(op))
    except (ValueError, TypeError, AttributeError):
        raise UsageError("operation ID must be a canonical UUID") from None
    if valid_op != op:
        raise UsageError("operation ID must be a canonical UUID")
    if getattr(args, "dry_run", False):
        exists = db.is_file()
        plan = {"dry_run": True, "would_apply": kind,
                "op_uuid": getattr(args, "op", None) or "(auto)",
                "payload_hash": ph, "store": str(db),
                "store_would_be_created": not exists, "body": body,
                "note": "no effect taken; no outbox intent written"}
        if needs_store and not exists:
            raise StoreAbsent(f"no ledger store at {db}; nothing to {kind}")
        return None, plan, ["dry-run: would apply %s (op %s, hash %s) — no effect taken"
                            % (kind, plan["op_uuid"], ph[:12])]
    ledger = Ledger.ensure_store(db) if kind == "entry.create" else Ledger.open_rw(db)
    try:
        ident = ledger.identity()
        ledger.record_intent(op, kind, ph)  # durable BEFORE mutation
        result = ledger.apply(op, kind, entry_uuid, body, actor)
        result["intent"] = ("recorded-then-settled" if getattr(args, "op", None)
                            else "auto-recorded-then-settled")
        return ident, result, None
    finally:
        ledger.close()


# ---------- commands ----------

def cmd_create(args):
    payload = read_payload(args)
    op = getattr(args, "op", None) or str(uuid_mod.uuid4())
    # entry UUID derived deterministically from the op UUID so a replay of the
    # same op binds to the identical entry and canonical payload.
    entry_uuid = str(uuid_mod.uuid5(ENTRY_UUID_NS, op))
    body = {"title": args.title, "payload": payload}
    ident, result, human = run_mutation(args, "entry.create", entry_uuid, body, operation_uuid=op)
    if human:
        return None, {"verification": {"effective": "UNKNOWN",
                                       "policy": "actor-reported only"}, **result}, human
    result["verification"] = {"effective": "UNKNOWN", "policy": "actor-reported only"}
    st = result.get("status")
    if st == "duplicate":
        orig = result.get("original", {})
        human = ["duplicate: this op already created entry %s (rev %s) at %s; "
                 "prior result replayed, no second entry"
                 % (orig.get("slug"), orig.get("revision"), orig.get("recorded_at"))]
    elif st == "not_applied":
        human = ["create NOT applied (%s); retained as a proposal" % result.get("reason")]
    else:
        human = ["created entry %s (rev %d, ledger %s)"
                 % (result.get("slug"), result.get("revision", 1), ident["slug"])]
    return ident, result, human


def _resolve_for_mutation(args):
    db = target_db(args)
    ledger = Ledger.open_read(db)
    try:
        row = ledger.entry_or_fail(args.entry)
        return row
    finally:
        ledger.close()


def cmd_note(args):
    row = _resolve_for_mutation(args)
    body = {"text": read_text(args)}
    ident, result, human = run_mutation(args, "entry.note", row["uuid"], body, needs_store=True)
    if human:
        return None, result, human
    st = result.get("status")
    if st == "duplicate":
        # replay of an op whose note was already durably recorded: the prior
        # result (not this call) carries note_seq; never KeyError, never re-add.
        orig = result.get("original", {})
        human = ["duplicate: note already recorded as #%s (entry %s) at %s; "
                 "prior result replayed, no second note appended"
                 % (orig.get("note_seq", "?"), row["slug"], orig.get("recorded_at", "?"))]
    elif st == "not_applied":
        human = ["note NOT applied (%s); retained as a proposal" % result.get("reason")]
    else:
        human = ["noted %s #%d (entry %s)" % (row["slug"], result["note_seq"], row["slug"])]
    return ident, result, human


def cmd_edit(args):
    row = _resolve_for_mutation(args)
    body = {"payload": read_payload(args, required=True),
            "expected_revision": args.expected_revision}
    ident, result, human = run_mutation(args, "entry.edit", row["uuid"], body, needs_store=True)
    if human:
        return None, result, human
    st = result.get("status")
    if st == "duplicate":
        orig = result.get("original", {})
        if orig.get("status") == "applied":
            human = ["duplicate: this op already applied the edit (%s now revision %s) "
                     "at %s; prior result replayed, nothing changed"
                     % (row["slug"], orig.get("revision"), orig.get("recorded_at"))]
        else:
            human = ["duplicate: this op's edit is retained as a NOT-applied stale "
                     "proposal (expected revision %s, current is %s); still not applied, "
                     "winner preserved" % (orig.get("expected_revision"),
                                           orig.get("current_revision"))]
    elif st == "applied":
        human = ["edit applied: %s now revision %d" % (row["slug"], result["revision"])]
    else:
        human = ["edit NOT applied (retained as stale proposal): expected revision %d, "
                 "current is %d; winner preserved" % (result["expected_revision"],
                                                      result["current_revision"])]
    return ident, result, human


def cmd_list(args):
    ledger = Ledger.open_read(target_db(args))
    try:
        ident = ledger.identity()
        rows, truncated = ledger.list_entries(min(args.limit, 500))
        entries = [{"uuid": r["uuid"], "slug": r["slug"], "title": r["title"],
                    "status": r["status"], "revision": r["revision"],
                    "created_at": r["created_at"], "updated_at": r["updated_at"]}
                   for r in rows]
        human = [f"ledger {ident['slug']} ({ident['uuid']}) — {len(entries)} entr"
                 f"{'y' if len(entries) == 1 else 'ies'}"
                 + (" (truncated)" if truncated else "")]
        human += h_table(["SLUG", "REV", "STATUS", "UPDATED", "TITLE"],
                         [[e["slug"], str(e["revision"]), e["status"],
                           e["updated_at"], e["title"]] for e in entries])
        if truncated:
            human.append("…raising --limit shows more; output is always bounded")
        data = {"count": len(entries), "truncated": truncated, "entries": entries}
    finally:
        ledger.close()
    return ident, data, human


def _entry_details(ledger, args):
    row = ledger.entry_or_fail(args.entry)
    notes, total = ledger.notes_for(row["uuid"], MAX_RESUME_NOTES)
    payload = json.loads(row["payload"])
    data = {
        "entry": {"uuid": row["uuid"], "slug": row["slug"], "title": row["title"],
                  "status": row["status"], "revision": row["revision"],
                  "created_at": row["created_at"], "updated_at": row["updated_at"],
                  "payload": payload,
                  "payload_status": "actor-reported; effective verification UNKNOWN"},
        "verification": {"effective": "UNKNOWN",
                         "policy": ("actor-reported only; a payload or note claiming "
                                    "'verified' cannot self-certify")},
        "notes": {"total": total, "returned": len(notes),
                  "items": [{"note_seq": n["note_seq"], "text": n["text"],
                             "recorded_at": n["recorded_at"],
                             "actor": json.loads(n["actor"])} for n in notes]},
    }
    human = h_kv([("entry", row["slug"]), ("uuid", row["uuid"]),
                  ("title", row["title"]), ("status", row["status"]),
                  ("revision", row["revision"]), ("created", row["created_at"]),
                  ("updated", row["updated_at"]),
                  ("verification", "UNKNOWN (actor-reported only)")])
    human += ["payload (concise structured details; full values via --json):",
              payload_human(payload)]
    human += ["notes (%d of %d shown):" % (len(notes), total)]
    for n in notes:
        human.append("  #%d %s  %s" % (n["note_seq"], n["recorded_at"], esc(n["text"])))
    return data, human


def cmd_show(args):
    ledger = Ledger.open_read(target_db(args))
    try:
        ident = ledger.identity()
        data, human = _entry_details(ledger, args)
    finally:
        ledger.close()
    return ident, data, human


def cmd_history(args):
    ledger = Ledger.open_read(target_db(args))
    try:
        ident = ledger.identity()
        row = ledger.entry_or_fail(args.entry)
        evs = ledger.history_for(row["uuid"], min(args.limit, 500))
        events = [{"seq": e["seq"], "kind": e["kind"], "op_uuid": e["op_uuid"],
                   "applied": bool(e["applied"]), "recorded_at": e["recorded_at"],
                   "payload_hash": e["payload_hash"], "result": json.loads(e["result"]),
                   "body": json.loads(e["body"]), "actor": json.loads(e["actor"])}
                  for e in evs]
        human = [f"history {row['slug']} ({len(events)} events, bounded):"]
        human += h_table(["SEQ", "KIND", "APPLIED", "RECORDED", "OP"],
                         [[str(e["seq"]), e["kind"], "yes" if e["applied"] else "no",
                           e["recorded_at"], e["op_uuid"][:13] + "…"] for e in events])
    finally:
        ledger.close()
    return ident, {"count": len(events), "events": events}, human


def cmd_resume(args):
    ledger = Ledger.open_read(target_db(args))
    try:
        ident = ledger.identity()
        data = ledger.resume()
        data["actor_context"] = build_actor(args)
        data["generated_at"] = now_iso()
    finally:
        ledger.close()
    human = [f"resume ledger {ident['slug']} ({ident['uuid']})",
             "verification: UNKNOWN — all payload statuses are actor-reported"]
    for e in data["open_entries"]:
        human.append("  %s  rev %d  %s  [delegate:%s artifact:%s notes:%d/%d]"
                     % (e["slug"], e["revision"], esc(e["title"]),
                        "ref" if e["delegate_ref"] is not None else "-",
                        "ref" if e["artifact_ref"] is not None else "-",
                        e["notes"]["returned"], e["notes"]["total"]))
    if data["entries_truncated"]:
        human.append("  …entry list truncated at bound")
    return ident, data, human


def cmd_ops(args):
    ledger = Ledger.open_read(target_db(args))
    try:
        ident = ledger.identity()
        if getattr(args, "op", None):
            data = ledger.op_lookup(args.op)
            human = h_kv([(k, v) for k, v in data.items() if k != "recorded_result"])
            if "recorded_result" in data:
                human += ["recorded_result:", payload_human(data["recorded_result"])]
            return ident, data, human
        if getattr(args, "pending", False):
            rows = ledger.pending_intents(min(args.limit, 200))
            data = {"count": len(rows),
                    "pending": [{"op_uuid": r["op_uuid"], "kind": r["kind"],
                                 "payload_hash": r["payload_hash"],
                                 "created_at": r["created_at"],
                                 "committed": r["event_seq"] is not None}
                                for r in rows]}
            human = ["pending outbox intents (recovery lookup; no blind retry):"]
            human += h_table(["OP", "KIND", "COMMITTED?", "CREATED"],
                             [[p["op_uuid"], p["kind"],
                               "yes (replay returns prior result)" if p["committed"]
                               else "no (safe to replay identical payload)",
                               p["created_at"]] for p in data["pending"]])
            return ident, data, human
        evs = ledger.conn.execute(
            "SELECT seq,kind,applied,recorded_at,op_uuid FROM events"
            " ORDER BY seq DESC LIMIT ?", (min(args.limit, 200),)).fetchall()
        data = {"count": len(evs), "events": [dict(r) for r in evs]}
        human = ["recent operations (most recent first, bounded):"]
        human += h_table(["SEQ", "KIND", "APPLIED", "RECORDED", "OP"],
                         [[str(r["seq"]), r["kind"], "yes" if r["applied"] else "no",
                           r["recorded_at"], r["op_uuid"][:13] + "…"] for r in evs])
    finally:
        ledger.close()
    return ident, data, human


# ---------- parser ----------

def positive_limit(value):
    try:
        number = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError("limit must be a positive integer") from None
    if number < 1:
        raise argparse.ArgumentTypeError("limit must be a positive integer")
    return number


def build_parser():
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--db", metavar="PATH", default=argparse.SUPPRESS,
                        help="store path; must lie under a known logical root "
                             "(default: ~/.ygg/ledgers/%s; existing legacy stores retained)"
                             % DEFAULT_DB_NAME)
    common.add_argument("--json", action="store_true", default=argparse.SUPPRESS,
                        help="versioned JSON envelope on stdout (full values)")
    mut = argparse.ArgumentParser(add_help=False)
    mut.add_argument("--op", metavar="OP-UUID", default=argparse.SUPPRESS,
                     help="caller-known operation UUID (idempotency key); "
                          "auto-allocated ops are durably pre-recorded in the outbox")
    mut.add_argument("--session-label", metavar="LABEL", default=argparse.SUPPRESS,
                     help="harness session label; recorded as reported, never attested, "
                          "not an authorization grant")
    mut.add_argument("--dry-run", action="store_true", default=argparse.SUPPRESS,
                     help="validate and plan without any effect")
    p = argparse.ArgumentParser(prog="ygg ledger",
                                description="Local working contexts; notes remain reported claims",
                                epilog="examples (zero mutation):\n"
                                       "  ygg ledger create --title 'First idea' --payload '{\"hypothesis\":\"cache wins\"}'\n"
                                       "  ygg ledger note --entry amber-otter-ab12 --file observation.md\n"
                                       "  ygg ledger edit --entry amber-otter-ab12 --expected-revision 1 --payload '{\"v\":2}'\n"
                                       "  ygg ledger resume --json",
                                allow_abbrev=False,
                                parents=[common])
    sub = p.add_subparsers(dest="cmd")
    c = sub.add_parser("create", parents=[common, mut], allow_abbrev=False,
                       help="create an entry (initializes the store if absent)")
    c.add_argument("--title", required=True)
    c.add_argument("--payload", default=None, help="inline JSON object (strict)")
    c.add_argument("--payload-file", default=None, help="JSON file ('-' = stdin)")
    c.set_defaults(func=cmd_create)
    c = sub.add_parser("list", parents=[common], allow_abbrev=False,
                       help="list entries (bounded)")
    c.add_argument("--limit", type=positive_limit, default=DEFAULT_LIST_LIMIT)
    c.set_defaults(func=cmd_list)
    c = sub.add_parser("show", parents=[common], allow_abbrev=False,
                       help="show one entry (concise payload details, notes bounded)")
    c.add_argument("--entry", required=True, help="entry slug or UUID")
    c.set_defaults(func=cmd_show)
    c = sub.add_parser("note", parents=[common, mut], allow_abbrev=False,
                       help="append an immutable freeform note")
    c.add_argument("--entry", required=True, help="entry slug or UUID")
    c.add_argument("--text", default=None)
    c.add_argument("--file", default=None, help="note text file ('-' = stdin)")
    c.set_defaults(func=cmd_note)
    c = sub.add_parser("edit", parents=[common, mut], allow_abbrev=False,
                       help="replace payload with CAS on expected revision; "
                            "stale proposals are retained, never overwrite")
    c.add_argument("--entry", required=True)
    c.add_argument("--payload", default=None)
    c.add_argument("--payload-file", default=None)
    c.add_argument("--expected-revision", type=int, required=True)
    c.set_defaults(func=cmd_edit)
    c = sub.add_parser("history", parents=[common], allow_abbrev=False,
                       help="event history for one entry")
    c.add_argument("--entry", required=True)
    c.add_argument("--limit", type=positive_limit, default=DEFAULT_LIST_LIMIT)
    c.set_defaults(func=cmd_history)
    c = sub.add_parser("resume", parents=[common], allow_abbrev=False,
                       help="bounded read-only context packet (open entries, notes, "
                            "delegate/artifact refs, unverified status)")
    c.set_defaults(func=cmd_resume)
    c = sub.add_parser("ops", parents=[common], allow_abbrev=False,
                       help="operation lookup / pending outbox intents (lost-ACK recovery)")
    c.add_argument("--op", dest="op", default=argparse.SUPPRESS,
                   help="look up one operation UUID")
    c.add_argument("--pending", action="store_true", default=argparse.SUPPRESS,
                   help="list pending outbox intents")
    c.add_argument("--limit", type=positive_limit, default=DEFAULT_LIST_LIMIT)
    c.set_defaults(func=cmd_ops)
    return p


def effective_status(data):
    """Collapse a duplicate replay onto its original outcome: replaying an op
    whose recorded result was not_applied stays not_applied."""
    if not isinstance(data, dict):
        return None
    st = data.get("status")
    if st == "duplicate":
        orig = data.get("original")
        if isinstance(orig, dict) and orig.get("status") == "not_applied":
            return "not_applied"
    return st


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.cmd is None:
        parser.print_help(sys.stderr)
        print("error: no command given; help shown (no mutation performed)",
              file=sys.stderr)
        return 2
    as_json = getattr(args, "json", False)
    try:
        ident, data, human = args.func(args)
    except LedgerError as e:
        msg = str(e)
        print(f"error: {e.code}: {msg}", file=sys.stderr)
        if as_json:
            print(json.dumps(err_envelope(e.code, msg), ensure_ascii=False,
                             allow_nan=False))
        return EXIT.get(e.code, 1)
    if effective_status(data) == "not_applied":
        # retained stale proposal (fresh or replayed): never reported as
        # applied / ok:true / exit 0, even though the event is durably recorded
        msg = "; ".join(human) if human else "operation recorded but NOT applied"
        if as_json:
            env = err_envelope(NOT_APPLIED_CODE, msg)
            env["error"]["recorded_result"] = data
            env["ledger"] = ident
            print(json.dumps(env, ensure_ascii=False, allow_nan=False))
        else:
            for line in human or []:
                print(line)
            print(f"error: {NOT_APPLIED_CODE}: proposal retained, NOT applied; "
                  "exit is non-zero so this can never read as success", file=sys.stderr)
        return EXIT[NOT_APPLIED_CODE]
    if as_json:
        print(json.dumps(ok_envelope(ident, data), ensure_ascii=False,
                         allow_nan=False))
    else:
        for line in human or []:
            print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
