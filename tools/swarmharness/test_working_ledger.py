#!/usr/bin/env python3
"""test_working_ledger.py — behavioral tests for the working-ledger preview.

All stores and temp files live under tempfile.mkdtemp() (honors the provided
disk TMPDIR); HOME is redirected so no host data is touched. Tests exercise
real behavior through the CLI as independent processes and the library API —
they do not grep the source.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid as uuid_mod
from pathlib import Path

WORKDIR = Path(__file__).resolve().parent
sys.path.insert(0, str(WORKDIR))

import working_ledger as wl  # noqa: E402

CLI = WORKDIR / "working_ledger_cli.py"
ROOT_PERMS = 0o755  # pre-existing roots must NOT be chmodded by the tool


class LedgerTestBase(unittest.TestCase):
    def setUp(self):
        self.home = Path(tempfile.mkdtemp(prefix="wl-home-"))
        self.root = self.home / "root1" / "ledgers"  # arbitrary dir, NOT a known root
        self.root.mkdir(parents=True)
        os.chmod(self.root.parent, ROOT_PERMS)
        self.db = self.root / "t.sqlite3"
        self.base_env = {k: v for k, v in os.environ.items()
                         if k not in ("YGG_LEDGER_ROOT", "YGG_SESSION_LABEL")}
        self.base_env.update(HOME=str(self.home), PYTHONPATH=str(WORKDIR),
                             TMPDIR=str(self.home))

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def cli(self, *args, root=True, label=None, stdin=None):
        env = dict(self.base_env)
        if root:
            env["YGG_LEDGER_ROOT"] = str(self.root)
        if label:
            env["YGG_SESSION_LABEL"] = label
        return subprocess.run([sys.executable, str(CLI), *args], env=env,
                              input=stdin, capture_output=True, text=True, timeout=120)

    def jcli(self, *args, **kw):
        if "--json" not in args:
            args = (*args, "--json")  # JSON helper: force the envelope
        p = self.cli(*args, **kw)
        self.assertEqual(p.returncode, 0,
                         f"CLI failed: {p.returncode}\nstdout={p.stdout}\nstderr={p.stderr}")
        return json.loads(p.stdout)

    def mkstore(self):
        return self.jcli("--db", str(self.db), "create", "--title", "Seed entry",
                         "--payload", '{"kind": "widget", "n": 3}', "--json")

    def api(self):
        return wl.Ledger.ensure_store(self.db)

    # --- helpers shared by concurrency tests ---
    def spawn(self, cmds):
        procs = []
        for c in cmds:
            env = dict(self.base_env)
            env["YGG_LEDGER_ROOT"] = str(self.root)
            procs.append(subprocess.Popen([sys.executable, str(CLI), *c], env=env,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                          text=True))
        results = []
        for p in procs:
            out, err = p.communicate(timeout=120)
            results.append((p.returncode, out, err))
        return results


class TestBasics(LedgerTestBase):
    def test_create_list_show_roundtrip(self):
        j = self.mkstore()
        self.assertTrue(j["ok"])
        self.assertTrue(j["ledger"]["uuid"])
        self.assertIn("-", j["ledger"]["slug"])
        slug = j["data"]["slug"]
        self.assertEqual(j["data"]["verification"]["effective"], "UNKNOWN")
        j2 = self.jcli("--db", str(self.db), "list", "--json")
        self.assertEqual(j2["data"]["count"], 1)
        j3 = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(j3["data"]["entry"]["payload"], {"kind": "widget", "n": 3})
        self.assertEqual(j3["data"]["entry"]["revision"], 1)
        h = self.jcli("--db", str(self.db), "history", "--entry", slug, "--json")
        self.assertEqual(h["data"]["count"], 1)
        self.assertEqual(h["data"]["events"][0]["kind"], "entry.create")

    def test_actor_claims_never_verified(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        self.cli("--db", str(self.db), "note", "--entry", slug, "--text", "verified: true")
        self.cli("--db", str(self.db), "edit", "--entry", slug, "--expected-revision", "1",
                 "--payload", '{"verified": true, "status": "done"}')
        r = self.jcli("--db", str(self.db), "resume", "--json")
        self.assertEqual(r["data"]["verification"]["effective"], "UNKNOWN")
        self.assertEqual(r["data"]["open_entries"][0]["payload"]["verified"], True)
        self.assertFalse(r["data"]["actor_context"]["attested"])
        self.assertTrue(r["data"]["actor_context"]["reported"])
        s = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(s["data"]["verification"]["effective"], "UNKNOWN")

    def test_session_label_reported_not_attested(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        r = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json",
                      label="harness-session-42")
        actor = r["data"]["notes"]["items"][0]["actor"] if r["data"]["notes"]["items"] else None
        res = self.jcli("--db", str(self.db), "resume", "--json",
                        label="harness-session-42")
        self.assertEqual(res["data"]["actor_context"]["session_label"], "harness-session-42")
        self.assertEqual(res["data"]["actor_context"]["session_label_source"], "env")
        self.assertFalse(res["data"]["actor_context"]["session_label_attested"])
        if actor:
            self.assertEqual(actor["session_label"], "harness-session-42")

    def test_control_chars_escaped_human_full_json(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        raw = "line1\nline2\ttab\x01ctl é ☺"
        p = self.cli("--db", str(self.db), "note", "--entry", slug, "--file", "-",
                     stdin=raw)
        self.assertEqual(p.returncode, 0, p.stderr)
        human = self.cli("--db", str(self.db), "show", "--entry", slug)
        self.assertNotIn("\x01", human.stdout, "raw control byte must be escaped")
        self.assertIn("\\x01", human.stdout)
        self.assertNotIn("line1\nline2", human.stdout, "raw newline must be escaped")
        self.assertIn("line1\\nline2", human.stdout)
        j = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(j["data"]["notes"]["items"][0]["text"], raw)

    def test_dry_run_has_no_effect(self):
        p = self.cli("--db", str(self.db), "create", "--title", "Plan only", "--dry-run")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertFalse(self.db.exists(), "dry-run must not create the store")
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        p2 = self.cli("--db", str(self.db), "note", "--entry", slug, "--text", "x",
                      "--dry-run")
        self.assertEqual(p2.returncode, 0, p2.stderr)
        self.assertIn("no effect", p2.stdout)
        n = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(n["data"]["notes"]["total"], 0)

    def test_strict_json_and_caps(self):
        p = self.cli("--db", str(self.db), "create", "--title", "T", "--payload",
                     '{"a": 1, "a": 2}')
        self.assertEqual(p.returncode, 2)
        self.assertIn("duplicate", p.stderr)
        self.assertFalse(self.db.exists(), "rejected input must have no effect")
        p = self.cli("--db", str(self.db), "create", "--title", "T", "--payload",
                     '{"a": NaN}')
        self.assertEqual(p.returncode, 2)
        self.assertFalse(self.db.exists())
        big = '{"x": "' + "y" * (wl.MAX_EVENT_BYTES + 10) + '"}'
        p = self.cli("--db", str(self.db), "create", "--title", "T", "--payload", big)
        self.assertEqual(p.returncode, 2)
        self.assertFalse(self.db.exists())
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        p = self.cli("--db", str(self.db), "note", "--entry", slug, "--file", "-",
                     stdin="z" * (wl.MAX_NOTE_BYTES + 10))
        self.assertEqual(p.returncode, 2)
        self.assertEqual(self.jcli("--db", str(self.db), "show", "--entry", slug,
                                   "--json")["data"]["notes"]["total"], 0)


class TestIdempotencyAndRecovery(LedgerTestBase):
    def test_same_op_same_input_replays_prior_result(self):
        op = str(uuid_mod.uuid4())
        j1 = self.jcli("--db", str(self.db), "create", "--title", "Same", "--op", op)
        self.assertEqual(j1["data"]["status"], "applied")
        j2 = self.jcli("--db", str(self.db), "create", "--title", "Same", "--op", op)
        self.assertEqual(j2["data"]["status"], "duplicate")
        self.assertTrue(j2["data"]["replayed"])
        self.assertEqual(j2["data"]["original"]["slug"], j1["data"]["slug"])
        n = self.jcli("--db", str(self.db), "list", "--json")
        self.assertEqual(n["data"]["count"], 1)

    def test_same_op_different_payload_holds(self):
        op = str(uuid_mod.uuid4())
        self.jcli("--db", str(self.db), "create", "--title", "Original", "--op", op)
        p = self.cli("--db", str(self.db), "create", "--title", "DIFFERENT", "--op", op)
        self.assertEqual(p.returncode, 4)
        self.assertIn("held", p.stderr.lower())
        self.assertEqual(self.jcli("--db", str(self.db), "list", "--json")
                         ["data"]["count"], 1)

    def test_duplicate_note_replay_human_and_json_idempotent(self):
        # regression: a duplicate note used to KeyError on note_seq and die with
        # exit 1 + empty stdout AFTER the durable dedup; both output modes must
        # replay the recorded result idempotently and never append again.
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        op = str(uuid_mod.uuid4())
        j1 = self.jcli("--db", str(self.db), "note", "--entry", slug,
                       "--text", "once", "--op", op)
        self.assertEqual(j1["data"]["status"], "applied")
        self.assertEqual(j1["data"]["note_seq"], 1)
        p = self.cli("--db", str(self.db), "note", "--entry", slug,
                     "--text", "once", "--op", op)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("duplicate", p.stdout)
        self.assertNotIn("Traceback", p.stderr)
        j2 = self.jcli("--db", str(self.db), "note", "--entry", slug,
                       "--text", "once", "--op", op)
        self.assertTrue(j2["ok"])
        self.assertEqual(j2["data"]["status"], "duplicate")
        self.assertEqual(j2["data"]["original"]["note_seq"], 1)
        s = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(s["data"]["notes"]["total"], 1, "no second note appended")

    def test_duplicate_create_and_edit_replay_original_shape(self):
        op1, op2 = str(uuid_mod.uuid4()), str(uuid_mod.uuid4())
        self.jcli("--db", str(self.db), "create", "--title", "D", "--op", op1)
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        # create duplicate: human output must describe the prior result, not
        # render "created entry None" from the duplicate shape
        p = self.cli("--db", str(self.db), "create", "--title", "D", "--op", op1)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("duplicate", p.stdout)
        self.assertNotIn("None", p.stdout)
        # edit applied, then replayed: original shape carries revision
        j = self.jcli("--db", str(self.db), "edit", "--entry", slug,
                      "--expected-revision", "1", "--payload", '{"v": 2}', "--op", op2)
        self.assertEqual(j["data"]["status"], "applied")
        p2 = self.cli("--db", str(self.db), "edit", "--entry", slug,
                      "--expected-revision", "1", "--payload", '{"v": 2}', "--op", op2)
        self.assertEqual(p2.returncode, 0, p2.stderr)
        self.assertIn("duplicate", p2.stdout)
        j2 = self.jcli("--db", str(self.db), "edit", "--entry", slug,
                       "--expected-revision", "1", "--payload", '{"v": 2}', "--op", op2)
        self.assertEqual(j2["data"]["status"], "duplicate")
        self.assertEqual(j2["data"]["original"]["status"], "applied")
        self.assertEqual(j2["data"]["original"]["revision"], 2)
        self.assertEqual(self.jcli("--db", str(self.db), "list", "--json")
                         ["data"]["count"], 1)

    def test_stale_edit_exit7_never_ok_and_replay_stays_not_applied(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        op = str(uuid_mod.uuid4())
        argv = ["--db", str(self.db), "edit", "--entry", slug,
                "--expected-revision", "9", "--payload", '{"late": true}', "--op", op]
        p = self.cli(*argv, "--json")
        self.assertEqual(p.returncode, 7, "stale proposal must not exit 0")
        env = json.loads(p.stdout)
        self.assertFalse(env["ok"], "not_applied must never be ok:true")
        self.assertEqual(env["error"]["code"], "op_not_applied")
        self.assertEqual(env["error"]["recorded_result"]["status"], "not_applied")
        ph = self.cli(*argv)  # human mode; this call is a replay of the recorded stale op
        self.assertEqual(ph.returncode, 7)
        self.assertIn("stale proposal", ph.stdout)
        self.assertIn("op_not_applied", ph.stderr)
        # replaying the SAME op with identical input: still a retained stale
        # proposal — reported not applied, never silently ok/exit 0
        p2 = self.cli(*argv, "--json")
        self.assertEqual(p2.returncode, 7)
        env2 = json.loads(p2.stdout)
        self.assertEqual(env2["error"]["recorded_result"]["status"], "duplicate")
        self.assertEqual(env2["error"]["recorded_result"]["original"]["status"],
                         "not_applied")
        self.assertFalse(env2["ok"])
        # entry unchanged: revision 1, original payload, exactly one edit event
        s = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(s["data"]["entry"]["revision"], 1)
        self.assertEqual(s["data"]["entry"]["payload"], {"kind": "widget", "n": 3},
                         "original winner payload untouched by the stale proposal")
        h = self.jcli("--db", str(self.db), "history", "--entry", slug, "--json")
        self.assertEqual(h["data"]["count"], 2)
        self.assertFalse(h["data"]["events"][1]["applied"])

    def test_pending_intent_different_payload_holds_before_any_event(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        op = str(uuid_mod.uuid4())
        L = self.api()
        L.record_intent(op, "entry.note", "deadbeef" * 8)  # crash after intent
        L.close()
        # retry under the same op UUID but a DIFFERENT payload, with no event
        # yet: must hold, not mutate, and preserve the original intent
        p = self.cli("--db", str(self.db), "note", "--entry", slug,
                     "--text", "attempted reword", "--op", op)
        self.assertEqual(p.returncode, 4, p.stderr)
        self.assertIn("DIFFERENT", p.stderr)
        self.assertEqual(self.jcli("--db", str(self.db), "show", "--entry", slug,
                                   "--json")["data"]["notes"]["total"], 0)
        h = self.jcli("--db", str(self.db), "history", "--entry", slug, "--json")
        self.assertEqual(h["data"]["count"], 1, "no event written for the held op")
        pend = self.jcli("--db", str(self.db), "ops", "--pending", "--json")
        self.assertEqual(pend["data"]["count"], 1)
        self.assertEqual(pend["data"]["pending"][0]["op_uuid"], op)
        self.assertFalse(pend["data"]["pending"][0]["committed"])
        lk = self.jcli("--db", str(self.db), "ops", "--op", op, "--json")["data"]
        self.assertEqual(lk["intent_state"], "pending")
        self.assertEqual(lk["payload_hash"], "deadbeef" * 8, "original intent preserved")

    def test_fail_after_commit_lost_ack_exact_recovery(self):
        # intent committed, mutation committed, caller never saw the ACK:
        # replaying the SAME op returns the prior result exactly once.
        L = self.api()
        op = str(uuid_mod.uuid4())
        body = {"title": "Lost ack"}
        entry_uuid = str(uuid_mod.uuid5(wl.ENTRY_UUID_NS, op))
        body["payload"] = {}  # byte-identical to the CLI's canonical body
        canonical = wl.canonical_json({"kind": "entry.create",
                                       "entry_uuid": entry_uuid, "body": body})
        L.record_intent(op, "entry.create", wl.sha256_hex(canonical))
        r1 = L.apply(op, "entry.create", entry_uuid, body,
                     wl.actor_context("recovery-test", "flag"))
        L.close()
        # simulate crash before ACK: new process replays the same op
        j = self.jcli("--db", str(self.db), "create", "--title", "Lost ack", "--op", op)
        self.assertEqual(j["data"]["status"], "duplicate")
        self.assertEqual(j["data"]["original"]["slug"], r1["slug"])
        self.assertEqual(self.jcli("--db", str(self.db), "list", "--json")
                         ["data"]["count"], 1)

    def test_outbox_intent_before_mutation_and_lookup(self):
        # crash BETWEEN intent and mutation: pending intent, no result;
        # ops lookup tells the caller replay is safe (same op, same payload).
        L = self.api()
        op = str(uuid_mod.uuid4())
        L.record_intent(op, "entry.note", "deadbe00")
        L.close()
        j = self.jcli("--db", str(self.db), "ops", "--pending", "--json")
        self.assertEqual(j["data"]["count"], 1)
        self.assertFalse(j["data"]["pending"][0]["committed"])
        lk = self.jcli("--db", str(self.db), "ops", "--op", op, "--json")["data"]
        self.assertEqual(lk["intent_state"], "pending")
        self.assertIn("SAME op UUID", lk["guidance"])
        # a replay under this op with a DIFFERENT payload holds: the durable
        # intent is the caller's first word, even with no event yet
        held = self.cli("--db", str(self.db), "create", "--title", "Reworded",
                        "--op", op)
        self.assertEqual(held.returncode, 4)
        # proper lost-ACK recovery: replay the SAME op bound to the SAME
        # payload the intent was recorded for — resolves to a real result
        op2 = str(uuid_mod.uuid4())
        entry_uuid = str(uuid_mod.uuid5(wl.ENTRY_UUID_NS, op2))
        body = {"title": "After crash", "payload": {}}
        canonical = wl.canonical_json({"kind": "entry.create",
                                       "entry_uuid": entry_uuid, "body": body})
        L2 = self.api()
        L2.record_intent(op2, "entry.create", wl.sha256_hex(canonical))
        L2.close()
        j2 = self.jcli("--db", str(self.db), "create", "--title", "After crash",
                       "--op", op2)
        self.assertEqual(j2["data"]["status"], "applied")
        lk2 = self.jcli("--db", str(self.db), "ops", "--op", op2, "--json")["data"]
        self.assertEqual(lk2["recorded_result"]["status"], "applied")
        self.assertEqual(self.jcli("--db", str(self.db), "ops", "--pending",
                                   "--json")["data"]["count"], 1,
                         "the mismatched intent stays pending-held, preserved")
        unk = self.jcli("--db", str(self.db), "ops", "--op",
                        str(uuid_mod.uuid4()), "--json")["data"]
        self.assertEqual(unk["status"], "unknown_operation")

    def test_crash_mid_transaction_rolls_back(self):
        self.mkstore()
        L = self.api()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        row = L.entry_row(slug)
        ev_before = L.event_count()
        notes_before = L.conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0]

        def boom(_ledger, _where):
            raise RuntimeError("simulated crash")

        L.hooks.append(boom)
        with self.assertRaises(RuntimeError):
            L.apply(str(uuid_mod.uuid4()), "entry.note", row["uuid"],
                    {"text": "should roll back"}, wl.actor_context())
        L.hooks.clear()
        self.assertEqual(L.event_count(), ev_before, "no event may survive a crash")
        self.assertEqual(L.conn.execute("SELECT COUNT(*) FROM notes").fetchone()[0],
                         notes_before)
        # store still healthy and usable afterwards
        r = L.apply(str(uuid_mod.uuid4()), "entry.note", row["uuid"],
                    {"text": "works again"}, wl.actor_context())
        self.assertEqual(r["status"], "applied")
        L.close()


class TestConcurrency(LedgerTestBase):
    def test_disjoint_appends_from_two_processes_not_lost(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        cmds = [["--db", str(self.db), "note", "--entry", slug, "--text",
                 f"writer{w}-note{i}", "--op", f"{uuid_mod.uuid4()}"]
                for w in (0, 1) for i in range(4)]
        results = self.spawn(cmds)
        for rc, _out, err in results:
            self.assertEqual(rc, 0, err)
        s = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(s["data"]["notes"]["total"], 8, "every disjoint append survives")
        texts = {n["text"] for n in s["data"]["notes"]["items"]}
        self.assertEqual(len(texts), 8)

    def test_cas_stale_proposal_retained_no_winner_loss(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        op_a, op_b, op_n = (str(uuid_mod.uuid4()) for _ in range(3))
        cmds = [
            ["--db", str(self.db), "edit", "--entry", slug, "--expected-revision", "1",
             "--payload", '{"writer": "A"}', "--op", op_a, "--json"],
            ["--db", str(self.db), "edit", "--entry", slug, "--expected-revision", "1",
             "--payload", '{"writer": "B"}', "--op", op_b, "--json"],
            # disjoint note racing the same entry must never be lost either;
            # its result echoes the entry revision, so results are correlated
            # by operation UUID and kind, never by revision alone
            ["--db", str(self.db), "note", "--entry", slug, "--text", "during edit",
             "--op", op_n, "--json"],
        ]
        results = self.spawn(cmds)
        by_op = {}
        for rc, out, err in results:
            self.assertIn(rc, (0, 7), err)  # stale edit records as exit 7, not ok
            env = json.loads(out)
            d = env["data"] if env["ok"] else env["error"]["recorded_result"]
            by_op[d["op_uuid"]] = d
        self.assertEqual(set(by_op), {op_a, op_b, op_n})
        d_a, d_b, d_n = by_op[op_a], by_op[op_b], by_op[op_n]
        edits = [d for d in (d_a, d_b) if d["kind"] == "entry.edit"]
        self.assertEqual(len(edits), 2)
        winners = [d for d in edits if d["status"] == "applied"]
        self.assertEqual(len(winners), 1, "exactly one CAS edit winner")
        self.assertEqual(winners[0]["revision"], 2)
        stales = [d for d in edits if d["status"] == "not_applied"]
        self.assertEqual(len(stales), 1)
        self.assertEqual(stales[0]["reason"], "stale_revision")
        self.assertEqual(stales[0]["current_revision"], 2)
        # the racing note is a NOTE, applied as such; it must never be
        # counted as an edit winner despite echoing revision 2
        self.assertEqual(d_n["kind"], "entry.note")
        self.assertEqual(d_n["status"], "applied")
        self.assertEqual(d_n["note_seq"], 1)
        s = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(s["data"]["entry"]["revision"], 2)
        self.assertIn(s["data"]["entry"]["payload"].get("writer"), ("A", "B"),
                      "winner's payload intact, not clobbered")
        h = self.jcli("--db", str(self.db), "history", "--entry", slug, "--json")
        self.assertEqual(h["data"]["count"], 4)
        kinds = [(e["kind"], e["applied"]) for e in h["data"]["events"]]
        self.assertEqual(kinds.count(("entry.edit", False)), 1,
                         "stale proposal retained in history, marked not applied")
        self.assertEqual(kinds.count(("entry.edit", True)), 1)
        self.assertEqual(kinds.count(("entry.note", True)), 1, "disjoint append survived")

    def test_replay_across_processes_is_deduped(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        op = str(uuid_mod.uuid4())
        r1 = self.spawn([["--db", str(self.db), "note", "--entry", slug,
                          "--text", "once", "--op", op, "--json"]])
        r2 = self.spawn([["--db", str(self.db), "note", "--entry", slug,
                          "--text", "once", "--op", op, "--json"]])
        self.assertEqual(json.loads(r1[0][1])["data"]["status"], "applied")
        self.assertEqual(json.loads(r2[0][1])["data"]["status"], "duplicate")
        self.assertEqual(self.jcli("--db", str(self.db), "show", "--entry", slug,
                                   "--json")["data"]["notes"]["total"], 1)

    def test_parallel_first_create_yields_one_identity(self):
        # concurrent initialization of a MISSING store: schema + identity +
        # creator proof commit in one transaction under one write lock, so six
        # racing first-creates must yield exactly one ledger identity and all
        # six entries in that single ledger.
        cmds = [["--db", str(self.db), "create", "--title", f"par-{i}", "--json"]
                for i in range(6)]
        results = self.spawn(cmds)
        uuids = set()
        for rc, out, err in results:
            self.assertEqual(rc, 0, err)
            uuids.add(json.loads(out)["ledger"]["uuid"])
        self.assertEqual(len(uuids), 1, f"exactly one ledger identity: {uuids}")
        n = self.jcli("--db", str(self.db), "list", "--json")
        self.assertEqual(n["data"]["count"], 6)
        conn = __import__("sqlite3").connect(str(self.db))
        rows = conn.execute(
            "SELECT value FROM meta WHERE key='ledger_uuid'").fetchall()
        creator = conn.execute(
            "SELECT value FROM meta WHERE key='creator'").fetchone()
        conn.close()
        self.assertEqual(len(rows), 1, "no split identity in meta")
        self.assertEqual(rows[0][0], uuids.pop())
        self.assertIsNotNone(creator, "creator ownership proof recorded")
        proof = json.loads(creator[0])
        self.assertEqual(proof["source"], "os")
        self.assertTrue(proof["reported"])
        self.assertFalse(proof["attested"], "creator proof is OS-reported only")


class TestStorePolicy(LedgerTestBase):
    def test_read_purity_no_creation_no_wal(self):
        # default logical path, nothing exists: explicit absent, zero creation
        p = self.cli("list", "--json", root=False)
        self.assertEqual(p.returncode, 3)
        self.assertIn("store_absent", p.stdout)
        self.assertFalse((self.home / ".yggterm").exists())
        self.assertFalse((self.home / ".ygg").exists())
        # help and bare invocation are zero-mutation too
        h1 = self.cli("--help", root=False)
        self.assertEqual(h1.returncode, 0)
        h2 = self.cli(root=False)
        self.assertEqual(h2.returncode, 2)
        h3 = self.cli("list", "--help", root=False)
        self.assertEqual(h3.returncode, 0)
        self.assertFalse((self.home / ".yggterm").exists())
        self.assertFalse((self.home / ".ygg").exists())
        # explicit missing db: absent, parent not created
        p2 = self.cli("--db", str(self.root / "missing" / "x.sqlite3"),
                      "resume", "--json")
        self.assertEqual(p2.returncode, 3)
        self.assertFalse((self.root / "missing").exists())
        # existing store: repeated reads leave no WAL/journal debris
        self.mkstore()
        for _ in range(3):
            self.cli("--db", str(self.db), "list", "--json")
            self.cli("--db", str(self.db), "resume")
        leftovers = [f.name for f in self.root.iterdir()
                     if f.name.endswith(("-wal", "-shm", "-journal"))]
        self.assertEqual(leftovers, [])

    def test_existing_public_zero_byte_file_is_refused_without_adoption(self):
        self.db.touch()
        os.chmod(self.db, 0o644)
        result = self.cli("--db", str(self.db), "create", "--title", "Refused", "--json")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.db.read_bytes(), b"")
        self.assertEqual(self.db.stat().st_mode & 0o777, 0o644)

    def test_identityless_store_held_never_claimed(self):
        # tables without identity: a torn write by an older writer or a
        # foreign store — must be held as schema_unknown, never claimed by
        # a new creator, never repaired in place
        import sqlite3
        conn = sqlite3.connect(str(self.db))
        for stmt in wl._SCHEMA:
            conn.execute(stmt)
        conn.commit()
        conn.close()
        os.chmod(self.db, 0o600)
        before = self.db.read_bytes()
        p = self.cli("--db", str(self.db), "create", "--title", "Claim", "--json")
        self.assertEqual(p.returncode, 5)
        self.assertIn("schema_unknown", p.stdout)
        self.assertEqual(self.db.read_bytes(), before, "no adoption, no repair")
        conn = sqlite3.connect(str(self.db))
        meta = dict(conn.execute("SELECT key,value FROM meta").fetchall())
        conn.close()
        self.assertEqual(meta, {}, "no identity minted into a held store")

    def test_fifo_and_nonregular_file_rejected(self):
        fifo = self.root / "pipe.sqlite3"
        os.mkfifo(str(fifo))
        p = self.cli("--db", str(fifo), "create", "--title", "T")
        self.assertEqual(p.returncode, 2)
        self.assertIn("path_rejected", p.stderr)
        p2 = self.cli("--db", str(fifo), "list", "--json")
        self.assertEqual(p2.returncode, 2, "reads refuse special files without opening them")

    def test_overlong_payload_rejected_before_outbox(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        p = self.cli("--db", str(self.db), "note", "--entry", slug, "--file", "-",
                     stdin="z" * (wl.MAX_NOTE_BYTES + 10))
        self.assertEqual(p.returncode, 2)
        pend = self.jcli("--db", str(self.db), "ops", "--pending", "--json")
        self.assertEqual(pend["data"]["count"], 0, "no outbox intent for rejected input")
        s = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(s["data"]["notes"]["total"], 0)

    def test_payload_file_strict_json(self):
        dup = self.root / "dup.json"
        dup.write_text('{"a": 1, "a": 2}')
        p = self.cli("--db", str(self.db), "create", "--title", "T",
                     "--payload-file", str(dup))
        self.assertEqual(p.returncode, 2)
        self.assertIn("duplicate", p.stderr)
        self.assertFalse(self.db.exists())
        nan = self.root / "nan.json"
        nan.write_text('{"a": NaN}')
        p2 = self.cli("--db", str(self.db), "create", "--title", "T",
                      "--payload-file", str(nan))
        self.assertEqual(p2.returncode, 2)
        self.assertFalse(self.db.exists())
        good = self.root / "ok.json"
        good.write_text('{"a": 1}')
        j = self.jcli("--db", str(self.db), "create", "--title", "OK",
                      "--payload-file", str(good))
        self.assertEqual(j["data"]["status"], "applied")

    def test_human_payload_concise_and_bounded(self):
        self.mkstore()
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        long_val = "y" * 500
        self.jcli("--db", str(self.db), "edit", "--entry", slug,
                  "--expected-revision", "1",
                  "--payload", '{"alpha": 1, "beta": {"nested": [1, 2, 3]},'
                               ' "long": "%s"}' % long_val)
        h = self.cli("--db", str(self.db), "show", "--entry", slug)
        self.assertEqual(h.returncode, 0, h.stderr)
        self.assertIn("alpha: 1", h.stdout)
        self.assertIn("beta:", h.stdout)
        self.assertIn("nested:", h.stdout)
        self.assertNotIn(long_val, h.stdout, "long values truncated in human output")
        self.assertIn("chars>", h.stdout)
        j = self.jcli("--db", str(self.db), "show", "--entry", slug, "--json")
        self.assertEqual(j["data"]["entry"]["payload"]["long"], long_val,
                         "full values only via --json")

    def test_corrupt_store_holds_without_repair(self):
        self.mkstore()
        raw = self.db.read_bytes()
        self.db.write_bytes(raw[:15] + b"X" + raw[16:])  # break SQLite header
        corrupted = self.db.read_bytes()
        p = self.cli("--db", str(self.db), "list", "--json")
        self.assertEqual(p.returncode, 5)
        self.assertIn("store_corrupt", p.stdout)
        self.assertEqual(self.db.read_bytes(), corrupted, "file must not be touched")
        self.assertNotEqual(self.db.read_bytes()[:16], b"SQLite format 3\x00")

    def test_unknown_schema_holds_without_repair(self):
        import sqlite3
        conn = sqlite3.connect(str(self.db))
        conn.execute("CREATE TABLE unrelated(x TEXT)")
        conn.commit()
        conn.close()
        os.chmod(self.db, 0o600)
        before = self.db.read_bytes()
        p = self.cli("--db", str(self.db), "list", "--json")
        self.assertEqual(p.returncode, 5)
        self.assertIn("schema_unknown", p.stdout)
        self.assertEqual(self.db.read_bytes(), before, "no repair, no added tables")
        conn = sqlite3.connect(str(self.db))
        tables = {r[0] for r in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'").fetchall()}
        conn.close()
        self.assertEqual(tables, {"unrelated"})

    def test_path_policy_rejects_unknown_location(self):
        # inside HOME (an ancestor of known roots) but not a known root
        stray = self.home / "plain.sqlite3"
        p = self.cli("--db", str(stray), "create", "--title", "T")
        self.assertEqual(p.returncode, 2)
        self.assertIn("path_rejected", p.stderr)
        self.assertFalse(stray.exists())
        # symlink whose target escapes every known root
        outside = self.home / "outside.sqlite3"
        outside.write_bytes(b"")
        link = self.root / "link.sqlite3"
        link.symlink_to(outside)
        p2 = self.cli("--db", str(link), "list", "--json")
        self.assertEqual(p2.returncode, 2)
        # symlink pointing INSIDE a known root is fine
        self.mkstore()
        good = self.root / "alias-link.sqlite3"
        good.symlink_to(self.db)
        result = self.cli("--db", str(good), "list", "--json")
        self.assertEqual(result.returncode, 2, "DB file aliases are refused; logical root aliases remain supported")

    def test_root_alias_and_relocation_uuid_stability(self):
        # new store on the legacy ~/.ygg alias (no ~/.yggterm yet)
        legacy = self.home / ".ygg" / "ledgers"
        legacy.mkdir(parents=True, mode=ROOT_PERMS)
        j = self.jcli("create", "--title", "Legacy home", "--json", root=False)
        uid1 = j["ledger"]["uuid"]
        created = legacy / wl.DEFAULT_DB_NAME
        self.assertTrue(created.exists())
        self.assertEqual(os.stat(created).st_mode & 0o777, 0o600, "DB must be 0600")
        # relocation: approved move .ygg -> .yggterm on the same host
        modern = self.home / ".yggterm" / "ledgers"
        modern.mkdir(parents=True, mode=ROOT_PERMS)
        os.rename(created, modern / wl.DEFAULT_DB_NAME)
        j2 = self.jcli("list", "--json", root=False)
        self.assertEqual(j2["ledger"]["uuid"], uid1, "identity survives relocation")
        self.assertTrue((modern / wl.DEFAULT_DB_NAME).exists())
        # tool never chmods pre-existing roots
        self.assertEqual(os.stat(legacy).st_mode & 0o777, ROOT_PERMS)
        self.assertEqual(os.stat(modern).st_mode & 0o777, ROOT_PERMS)
        # fresh creation prefers ~/.yggterm when both exist
        j3 = self.jcli("create", "--title", "Modern home", "--json", root=False)
        self.assertTrue((modern / wl.DEFAULT_DB_NAME).exists())

    def test_new_store_dir_perms_and_resume_refs(self):
        self.mkstore()
        # pre-existing root dir must be untouched by the tool
        self.assertEqual(os.stat(self.root).st_mode & 0o777, ROOT_PERMS)
        # tool-created deep dirs are private:
        deep = self.root / "x" / "y"
        j = self.jcli("--db", str(deep / "b.sqlite3"), "create", "--title", "Deep")
        self.assertTrue(j["ok"])
        self.assertEqual(os.stat(deep).st_mode & 0o777, 0o700)
        self.assertEqual(os.stat(deep / "b.sqlite3").st_mode & 0o777, 0o600)
        slug = self.jcli("--db", str(self.db), "list", "--json")["data"]["entries"][0]["slug"]
        self.jcli("--db", str(self.db), "edit", "--entry", slug,
                  "--expected-revision", "1",
                  "--payload", '{"artifact_ref": {"kind": "log", "uri": "ref://log-9"},'
                               ' "delegate": {"job": "job-31"}, "status": "looks done"}')
        r = self.jcli("--db", str(self.db), "resume", "--json")
        e = r["data"]["open_entries"][0]
        self.assertEqual(e["artifact_ref"], {"kind": "log", "uri": "ref://log-9"})
        self.assertEqual(e["delegate_ref"], {"job": "job-31"})
        self.assertIn("never read", r["data"]["note"])
        self.assertEqual(e["payload_status"], "actor-reported; effective verification UNKNOWN")

    def test_missing_entry_and_flag_strictness(self):
        self.mkstore()
        p = self.cli("--db", str(self.db), "note", "--entry", "ghost-zzz",
                     "--text", "hi")
        self.assertEqual(p.returncode, 6)
        p2 = self.cli("--db", str(self.db), "create", "--title", "T", "--payl", "{}")
        self.assertEqual(p2.returncode, 2, "no flag abbreviation allowed")
        p3 = self.cli("--db", str(self.db), "note", "--entry",
                      self.jcli("--db", str(self.db), "list", "--json")
                      ["data"]["entries"][0]["slug"])
        self.assertEqual(p3.returncode, 2, "exactly one of --text/--file required")



class TestParentFaults(LedgerTestBase):
    def test_automatic_create_key_recovers_lost_first_ack(self):
        result = self.mkstore()
        replay = self.jcli("--db", str(self.db), "create", "--title", "Seed entry",
                           "--payload", '{"kind":"widget","n":3}',
                           "--op", result["data"]["op_uuid"])
        self.assertEqual(replay["data"]["status"], "duplicate")
        self.assertEqual(self.jcli("--db", str(self.db), "list")["data"]["count"], 1)

    def test_existing_empty_file_is_never_adopted(self):
        self.db.touch(mode=0o600)
        before = self.db.stat()
        result = self.cli("--db", str(self.db), "create", "--title", "Refused", "--json")
        self.assertEqual(result.returncode, 5, result.stderr)
        self.assertEqual(self.db.read_bytes(), b"")
        self.assertEqual(self.db.stat().st_mode, before.st_mode)

    def test_existing_public_file_is_not_chmodded_or_written(self):
        self.mkstore()
        os.chmod(self.db, 0o644)
        before = self.db.read_bytes()
        result = self.cli("--db", str(self.db), "create", "--title", "Refused", "--json")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.db.read_bytes(), before)
        self.assertEqual(self.db.stat().st_mode & 0o777, 0o644)

    def test_foreign_version_marker_does_not_authorize_schema_repair(self):
        import sqlite3
        conn = sqlite3.connect(self.db)
        conn.execute("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        conn.execute("INSERT INTO meta VALUES('schema_version','1')")
        conn.commit(); conn.close(); os.chmod(self.db, 0o600)
        before = self.db.read_bytes()
        result = self.cli("--db", str(self.db), "create", "--title", "Refused", "--json")
        self.assertEqual(result.returncode, 5)
        self.assertEqual(self.db.read_bytes(), before)

    def test_full_event_cap_rejects_before_first_store(self):
        payload = json.dumps({"x": "a" * (wl.MAX_EVENT_BYTES - 40)})
        self.assertLess(len(payload.encode()), wl.MAX_EVENT_BYTES)
        result = self.cli("--db", str(self.db), "create", "--title", "Refused", "--payload", payload)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertFalse(self.db.exists())

    def test_invalid_operation_is_refused_in_execution_and_planning(self):
        for suffix in [(), ("--dry-run",)]:
            result = self.cli("--db", str(self.db), "create", "--title", "Refused",
                              "--op", "not-a-uuid", *suffix)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(self.db.exists())

    def test_two_distinct_roots_need_explicit_selection(self):
        modern = self.home / ".ygg" / "ledgers"
        legacy = self.home / ".yggterm" / "ledgers"
        for root in [modern, legacy]:
            self.jcli("--db", str(root / wl.DEFAULT_DB_NAME), "create", "--title", "Context", root=False)
        result = self.cli("list", "--json", root=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("two distinct", result.stderr)

    def test_json_payload_stdin_works_without_an_input_file(self):
        result = self.jcli("--db", str(self.db), "create", "--title", "Piped",
                           "--payload-file", "-", stdin='{"task":"task-7"}')
        shown = self.jcli("--db", str(self.db), "show", "--entry", result["data"]["slug"])
        self.assertEqual(shown["data"]["entry"]["payload"], {"task": "task-7"})


class TestIndependentReviewFaults(LedgerTestBase):
    def test_negative_limits_rejected_before_reading_any_store(self):
        for cmd in [("history", "--entry", "missing"), ("ops",), ("ops", "--pending"), ("list",)]:
            result = self.cli(*cmd, "--limit", "-1", "--json")
            self.assertEqual(result.returncode, 2)
            self.assertFalse(self.db.exists())

    def test_oversized_revision_has_no_pending_intent(self):
        result = self.mkstore(); slug = result["data"]["slug"]
        for suffix in [(), ("--dry-run",)]:
            op = str(uuid_mod.uuid4())
            result = self.cli("--db", str(self.db), "edit", "--entry", slug,
                              "--expected-revision", str(1 << 63), "--payload", "{}",
                              "--op", op, "--json", *suffix)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(json.loads(result.stdout)["ok"])
            state = self.jcli("--db", str(self.db), "ops", "--op", op)
            self.assertEqual(state["data"]["status"], "unknown_operation")

    def test_wal_store_refused_without_coordination_files(self):
        import sqlite3
        self.mkstore()
        conn = sqlite3.connect(self.db)
        self.assertEqual(conn.execute("PRAGMA journal_mode=WAL").fetchone()[0], "wal")
        conn.close()
        before = {p.name: p.read_bytes() for p in self.root.iterdir() if p.is_file()}
        result = self.cli("--db", str(self.db), "list", "--json")
        self.assertEqual(result.returncode, 5, result.stderr)
        after = {p.name: p.read_bytes() for p in self.root.iterdir() if p.is_file()}
        self.assertEqual(after, before)

    def test_untrusted_actor_label_rejected_before_create_or_note(self):
        long_label = "x" * 70000
        result = self.cli("--db", str(self.db), "create", "--title", "Refused",
                          "--session-label", long_label, "--json")
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.db.exists())
        created = self.mkstore()
        before = self.db.read_bytes()
        result = self.cli("--db", str(self.db), "note", "--entry", created["data"]["slug"],
                          "--text", "short", "--json", label=long_label)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.db.read_bytes(), before)

    def test_overflow_number_and_invalid_unicode_return_json_refusals(self):
        for payload in ['{"number":1e999}', r'{"text":"\ud800"}']:
            result = self.cli("--db", str(self.db), "create", "--title", "Refused",
                              "--payload", payload, "--json")
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(json.loads(result.stdout)["ok"])
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse(self.db.exists())

if __name__ == "__main__":
    unittest.main(verbosity=2)
