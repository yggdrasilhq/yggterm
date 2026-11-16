#!/usr/bin/env python3
"""ygg-auth lineage law: one writer refreshes, readers hold access only, a dead writer
grant moves the lineage to the newest live copy, and sign-in is refused while any host
refreshed the account inside the 30-day window.

    python3 tests/test_ygg_auth_lineage.py [--script <path to ygg-auth.py>]

Isolation: HOME, YGG_AUTH_HOME and CODEX_HOME point at a temp dir before the script is
imported (it reads them at import). No network: the refresh grant, the ssh pull and the
fleet evidence are replaced, and each screen asserts whether they were called.

Falsified, not merely passed:
  • the rotation screen fails if a read of a fresh writer calls the refresh grant;
  • the reader screen fails if a reader copy still carries a refresh grant;
  • the recovery screen fails if recovery presents an OLDER copy's grant.
"""
import base64
import importlib.util
import json
import os
import sys
import tempfile
import time
from datetime import datetime, timezone

ARGS = sys.argv[1:]
if "--script" in ARGS:
    SCRIPT = os.path.abspath(ARGS[ARGS.index("--script") + 1])
else:
    SCRIPT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "ygg-auth.py"))

TMP = tempfile.mkdtemp(prefix="ygg-lineage-test-")
os.environ["HOME"] = os.path.join(TMP, "home")
os.makedirs(os.environ["HOME"])
os.environ["YGG_AUTH_HOME"] = os.path.join(TMP, "store")
os.environ["CODEX_HOME"] = os.path.join(os.environ["HOME"], ".codex")
os.makedirs(os.environ["CODEX_HOME"])

_spec = importlib.util.spec_from_file_location("ygg_auth_under_test", SCRIPT)
ya = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ya)
# The real roster lives under the user's store, which the sandbox does not use. Screens
# that exercise the refresh gate override it with an unknown or partial roster themselves.
ya.load_fleet_hosts = lambda: [ya.HOSTNAME, "host-a", "host-b"]

NOW = time.time()
DAY = 86400
SLUG = "juju_example.test"
ACCOUNT = "acct-test-0001"
PASS, FAIL = [], []


def b64u(obj):
    return base64.urlsafe_b64encode(json.dumps(obj).encode()).rstrip(b"=").decode()


def jwt(claims):
    return f"{b64u({'alg': 'none', 'typ': 'JWT'})}.{b64u(claims)}.sig"


def iso(epoch):
    return datetime.fromtimestamp(epoch, timezone.utc).isoformat()


def record(rt, access_in_days, refreshed_days_ago):
    """A codex auth record with the claim shapes identity_of() reads."""
    return {
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": None,
        "tokens": {
            "id_token": jwt({"email": "juju@example.test",
                             "https://api.openai.com/auth": {"chatgpt_account_id": ACCOUNT,
                                                             "chatgpt_plan_type": "plus"}}),
            "access_token": jwt({"exp": int(NOW + access_in_days * DAY),
                                 "https://api.openai.com/auth": {"chatgpt_account_id": ACCOUNT}}),
            "refresh_token": rt,
            "account_id": ACCOUNT,
        },
        "last_refresh": iso(NOW - refreshed_days_ago * DAY),
    }


def put_store(rec):
    os.makedirs(ya.store_dir("codex"), exist_ok=True)
    ya.atomic_write_json(os.path.join(ya.store_dir("codex"), SLUG + ".json"), rec)
    ya.stamp_provenance("codex", SLUG, record=rec, writer=ya.HOSTNAME, source="capture",
                        writer_updated_at=time.time())


def read_store():
    return ya.read_json(os.path.join(ya.store_dir("codex"), SLUG + ".json"))


def reset():
    import shutil
    shutil.rmtree(ya.store_dir("codex"), ignore_errors=True)
    live = ya.HARNESSES["codex"]["auth_file"]("codex")
    if os.path.exists(live):
        os.remove(live)


def screen(name):
    def deco(fn):
        try:
            fn()
            PASS.append(name)
            print(f"PASS  {name}")
        except AssertionError as exc:
            FAIL.append(name)
            print(f"FAIL  {name}: {exc}")
        except Exception as exc:  # a crash is a failure, not a skip
            FAIL.append(name)
            print(f"FAIL  {name}: {type(exc).__name__}: {exc}")
        return fn
    return deco


def expect_auth_error(fn, reason=None):
    try:
        fn()
    except ya.AuthError as exc:
        if reason is not None:
            assert exc.details.get("reason") == reason, f"reason {exc.details.get('reason')!r} != {reason!r}"
        return exc
    raise AssertionError("expected AuthError, got success")


@screen("reader copy never carries a refresh grant")
def _():
    reset()
    bundle = ya.access_bundle(record("RT-SECRET-A", 10, 1))
    r = ya.reader_record(bundle, "host-a")
    assert ya.is_reader_record(r), "reader record not recognised as a reader"
    assert "RT-SECRET-A" not in json.dumps(r), "a refresh grant leaked into a reader copy"
    assert r["tokens"]["refresh_token"] == ya.READER_RT_PREFIX + "host-a"


@screen("writer does NOT call the refresh grant for a fresh access token (rotation screen)")
def _():
    reset()
    put_store(record("RT-W1", 10, 1))
    real = ya.refresh_grant

    def must_not_run(_record):
        raise AssertionError("refresh grant called although the access token has 10 days left")

    ya.refresh_grant = must_not_run
    try:
        bundle, refreshed = ya._writer_access("codex", SLUG)
    finally:
        ya.refresh_grant = real
    assert refreshed is False
    assert read_store()["tokens"]["refresh_token"] == "RT-W1", "a read rotated the grant"
    assert "refresh_token" not in bundle, "the bundle carried a refresh grant"


@screen("writer refreshes near expiry and persists the rotated grant")
def _():
    reset()
    put_store(record("RT-W1", 1, 7))
    real = ya.refresh_grant

    def rotated(rec):
        out = json.loads(json.dumps(rec))
        out["tokens"]["refresh_token"] = "RT-W2"
        out["tokens"]["access_token"] = record("x", 10, 0)["tokens"]["access_token"]
        out["last_refresh"] = iso(NOW)
        return out

    ya.refresh_grant = rotated
    real_evidence, real_hosts = ya.fleet_evidence, ya.load_fleet_hosts
    ya.fleet_evidence = lambda _h, _s, _b: [
        {"host": ya.HOSTNAME, "kind": "writer", "last_refresh_epoch": NOW - 7 * DAY},
        {"host": "host-b", "kind": "reader", "writer": ya.HOSTNAME}]
    ya.load_fleet_hosts = lambda: [ya.HOSTNAME, "host-b"]
    try:
        bundle, refreshed = ya._writer_access("codex", SLUG)
    finally:
        ya.refresh_grant, ya.fleet_evidence, ya.load_fleet_hosts = real, real_evidence, real_hosts
    assert refreshed is True
    assert read_store()["tokens"]["refresh_token"] == "RT-W2", "rotated grant was not persisted"
    prov = ya.load_provenance("codex")[SLUG]
    assert prov.get("role") == "writer" and prov.get("source") == "refresh"


def _held_refresh(evidence_rows, roster):
    """Run a near-expiry writer read with the fleet view stubbed. Returns the reason the
    refresh was held, and fails the screen if the provider was called anyway."""
    reset()
    put_store(record("RT-GATED", 1, 7))
    real_refresh, real_evidence, real_hosts = ya.refresh_grant, ya.fleet_evidence, ya.load_fleet_hosts

    def must_not_present(_rec):
        raise AssertionError("the refresh grant was presented without a proven-newest fleet view")

    ya.refresh_grant = must_not_present
    ya.fleet_evidence = lambda _h, _s, _b: evidence_rows
    ya.load_fleet_hosts = lambda: roster
    try:
        exc = expect_auth_error(lambda: ya._writer_access("codex", SLUG), "held_unproven")
    finally:
        ya.refresh_grant, ya.fleet_evidence, ya.load_fleet_hosts = real_refresh, real_evidence, real_hosts
    assert read_store()["tokens"]["refresh_token"] == "RT-GATED", "a held grant was rotated anyway"
    return str(exc)


@screen("a grant the provider refused is NOT presented again (no repeat refusals)")
def _():
    reset()
    put_store(record("RT-REFUSED", 1, 7))
    real_refresh, real_evidence = ya.refresh_grant, ya.fleet_evidence
    calls = []

    def dead(_rec):
        calls.append(1)
        raise ya.AuthError(5, "refresh grant rejected: HTTP 401", {"reason": "refresh_invalid"})

    ya.refresh_grant = dead
    ya.fleet_evidence = lambda _h, _s, _b: [{"host": ya.HOSTNAME, "kind": "writer",
                                             "last_refresh_epoch": NOW - 7 * DAY},
                                            {"host": "host-b", "kind": "reader", "writer": ya.HOSTNAME}]
    try:
        expect_auth_error(lambda: ya._writer_access("codex", SLUG), "refresh_invalid")
        expect_auth_error(lambda: ya._writer_access("codex", SLUG), "reauth_required")
    finally:
        ya.refresh_grant, ya.fleet_evidence = real_refresh, real_evidence
    assert len(calls) == 1, f"the refused grant was presented {len(calls)} times"


@screen("refresh is HELD when the fleet roster is unknown (never presents on a guess)")
def _():
    why = _held_refresh([{"host": ya.HOSTNAME, "kind": "writer", "last_refresh_epoch": NOW - 7 * DAY}], [])
    assert "roster missing" in why, why


@screen("refresh is HELD while a peer is unreachable")
def _():
    why = _held_refresh([{"host": ya.HOSTNAME, "kind": "writer", "last_refresh_epoch": NOW - 7 * DAY},
                         {"host": "host-b", "kind": "unreachable"}],
                        [ya.HOSTNAME, "host-b"])
    assert "unreachable" in why, why


@screen("refresh is HELD when a peer holds a NEWER grant (this one is superseded)")
def _():
    why = _held_refresh([{"host": ya.HOSTNAME, "kind": "writer", "last_refresh_epoch": NOW - 7 * DAY},
                         {"host": "host-a", "kind": "writer", "last_refresh_epoch": NOW - 1 * DAY}],
                        [ya.HOSTNAME, "host-a"])
    assert "superseded" in why, why


@screen("the refresh grant refuses a reader copy before any network call")
def _():
    reset()
    reader = ya.reader_record(ya.access_bundle(record("RT-X", 10, 1)), "host-a")
    real_urlopen = ya.urllib.request.urlopen

    def no_network(*_a, **_k):
        raise AssertionError("network touched for a reader copy")

    ya.urllib.request.urlopen = no_network
    try:
        exc = expect_auth_error(lambda: ya.refresh_grant(reader), "not_writer")
    finally:
        ya.urllib.request.urlopen = real_urlopen
    assert "ygg auth sync" in str(exc)


@screen("apply refuses to demote a writer grant NEWER than the bundle")
def _():
    reset()
    put_store(record("RT-NEW-LOCAL", 10, 1))           # refreshed 1 day ago
    bundle = ya.access_bundle(record("RT-REMOTE", 10, 5))  # remote is 5 days old
    expect_auth_error(lambda: ya.apply_reader_bundle("codex", SLUG, bundle, "host-a"), "local_writer_newer")
    assert read_store()["tokens"]["refresh_token"] == "RT-NEW-LOCAL", "a newer local grant was demoted"


@screen("apply demotes an OLDER writer copy to a reader (the dead-copy case)")
def _():
    reset()
    put_store(record("RT-DEAD-OLD", 1, 20))            # refreshed 20 days ago
    bundle = ya.access_bundle(record("RT-LIVE", 10, 1))
    ya.apply_reader_bundle("codex", SLUG, bundle, "host-a")
    stored = read_store()
    assert ya.is_reader_record(stored), "older copy was not converted"
    assert "RT-DEAD-OLD" not in json.dumps(stored) and "RT-LIVE" not in json.dumps(stored)
    prov = ya.load_provenance("codex")[SLUG]
    assert prov.get("writer") == "host-a" and prov.get("role") == "reader"


@screen("a writer copy superseded by a NEWER grant elsewhere is never presented (rotation screen)")
def _():
    reset()
    put_store(record("RT-ROTATED-OUT", 1, 20))         # older than the fleet's newest grant
    real_refresh, real_pull, real_evidence = ya.refresh_grant, ya.pull_access, ya.fleet_evidence
    asked = []

    def must_not_present(_rec):
        raise AssertionError("a rotated-out grant was presented to the provider")

    def evidence(_h, _s, _b):
        return [
            {"host": ya.HOSTNAME, "kind": "writer", "last_refresh_epoch": NOW - 20 * DAY},
            {"host": "host-b", "kind": "writer", "last_refresh_epoch": NOW - 30 * DAY},
            {"host": "host-a", "kind": "writer", "last_refresh_epoch": NOW - 1 * DAY},
        ]

    def pull(_h, _s, writer, _b):
        asked.append(writer)
        return ya.access_bundle(record("RT-LIVE-A", 10, 1))

    ya.refresh_grant, ya.pull_access, ya.fleet_evidence = must_not_present, pull, evidence
    try:
        out = ya.sync_lineage("codex", SLUG, b"")
    finally:
        ya.refresh_grant, ya.pull_access, ya.fleet_evidence = real_refresh, real_pull, real_evidence
    assert out["state"] == "superseded_moved" and out["writer"] == "host-a", out
    assert asked == ["host-a"], f"pulled more than the newest copy: {asked}"
    assert ya.is_reader_record(read_store()), "superseded copy was not demoted"


@screen("recovery never pulls a copy OLDER than the grant that just died")
def _():
    reset()
    put_store(record("RT-NEWEST-DEAD", 1, 1))          # this host holds the newest grant, and it is dead
    real_refresh, real_pull, real_evidence = ya.refresh_grant, ya.pull_access, ya.fleet_evidence
    asked = []

    def dead(_rec):
        raise ya.AuthError(5, "refresh grant rejected: HTTP 401", {"reason": "refresh_invalid"})

    def pull(_h, _s, writer, _b):
        asked.append(writer)
        raise AssertionError("an older, rotated-out copy was presented")

    ya.refresh_grant, ya.pull_access = dead, pull
    ya.fleet_evidence = lambda _h, _s, _b: [
        {"host": ya.HOSTNAME, "kind": "writer", "last_refresh_epoch": NOW - 1 * DAY},
        {"host": "host-b", "kind": "writer", "last_refresh_epoch": NOW - 20 * DAY}]
    try:
        expect_auth_error(lambda: ya.sync_lineage("codex", SLUG, b""), "refused_inside_window")
    finally:
        ya.refresh_grant, ya.pull_access, ya.fleet_evidence = real_refresh, real_pull, real_evidence
    assert asked == [], f"an older copy was presented: {asked}"


@screen("a reader whose writer died moves to a NEWER live copy elsewhere (no sign-in)")
def _():
    reset()
    ya.apply_reader_bundle("codex", SLUG, ya.access_bundle(record("RT-OLD-WRITER", -1, 20)), "host-x")
    real_pull, real_evidence = ya.pull_access, ya.fleet_evidence
    asked = []

    def pull(_h, _s, writer, _b):
        asked.append(writer)
        if writer == "host-x":
            raise ya.AuthError(5, "could not pull: refresh grant rejected: HTTP 401", {"reason": "pull_failed"})
        return ya.access_bundle(record("RT-LIVE-A", 10, 1))

    ya.pull_access = pull
    ya.fleet_evidence = lambda _h, _s, _b: [
        {"host": ya.HOSTNAME, "kind": "reader", "writer": "host-x"},
        {"host": "host-x", "kind": "writer", "last_refresh_epoch": NOW - 20 * DAY},
        {"host": "host-a", "kind": "writer", "last_refresh_epoch": NOW - 1 * DAY}]
    try:
        out = ya.sync_lineage("codex", SLUG, b"")
    finally:
        ya.pull_access, ya.fleet_evidence = real_pull, real_evidence
    assert out["state"] == "lineage_moved" and out["writer"] == "host-a", out
    assert asked == ["host-x", "host-a"], f"unexpected pull order: {asked}"
    assert ya.load_provenance("codex")[SLUG].get("writer") == "host-a"


@screen("no live copy anywhere -> sign-in is the only path (idle 40 days)")
def _():
    reset()
    put_store(record("RT-DEAD", 1, 40))
    real_refresh, real_evidence = ya.refresh_grant, ya.fleet_evidence

    def dead(_rec):
        raise ya.AuthError(5, "refresh grant rejected: HTTP 401", {"reason": "refresh_invalid"})

    ya.refresh_grant = dead
    ya.fleet_evidence = lambda _h, _s, _b: [{"host": ya.HOSTNAME, "kind": "writer",
                                             "last_refresh_epoch": NOW - 40 * DAY}]
    try:
        exc = expect_auth_error(lambda: ya.sync_lineage("codex", SLUG, b""), "reauth_required")
    finally:
        ya.refresh_grant, ya.fleet_evidence = real_refresh, real_evidence
    assert "sign-in" in str(exc).lower()


@screen("a grant refused INSIDE 30 days is investigated, not signed in")
def _():
    reset()
    put_store(record("RT-DEAD", 1, 2))
    real_refresh, real_evidence = ya.refresh_grant, ya.fleet_evidence

    def dead(_rec):
        raise ya.AuthError(5, "refresh grant rejected: HTTP 401", {"reason": "refresh_invalid"})

    ya.refresh_grant = dead
    ya.fleet_evidence = lambda _h, _s, _b: [{"host": ya.HOSTNAME, "kind": "writer",
                                             "last_refresh_epoch": NOW - 2 * DAY}]
    try:
        expect_auth_error(lambda: ya.sync_lineage("codex", SLUG, b""), "refused_inside_window")
    finally:
        ya.refresh_grant, ya.fleet_evidence = real_refresh, real_evidence


@screen("sign-in gate: refused while the lineage refreshed within 30 days")
def _():
    real_evidence = ya.fleet_evidence
    ya.fleet_evidence = lambda _h, _s, _b: [
        {"host": "host-a", "kind": "writer", "last_refresh_epoch": NOW - 2 * DAY}]
    try:
        expect_auth_error(lambda: ya._sign_in_gate("codex", SLUG + "@x"), "lineage_live")
    finally:
        ya.fleet_evidence = real_evidence


@screen("sign-in gate: allowed only after 30 idle days on every host")
def _():
    real_evidence = ya.fleet_evidence
    ya.fleet_evidence = lambda _h, _s, _b: [
        {"host": "host-a", "kind": "writer", "last_refresh_epoch": NOW - 40 * DAY},
        {"host": ya.HOSTNAME, "kind": "absent"}]
    try:
        ya._sign_in_gate("codex", "juju@example.test")
    finally:
        ya.fleet_evidence = real_evidence


@screen("sign-in gate: refused while any host cannot be read (cannot prove absence)")
def _():
    real_evidence = ya.fleet_evidence
    ya.fleet_evidence = lambda _h, _s, _b: [{"host": "host-b", "kind": "unreachable"},
                                            {"host": ya.HOSTNAME, "kind": "absent"}]
    try:
        expect_auth_error(lambda: ya._sign_in_gate("codex", "juju@example.test"), "lineage_unproven")
    finally:
        ya.fleet_evidence = real_evidence


@screen("sign-in gate: --name is required so the fleet can be checked first")
def _():
    expect_auth_error(lambda: ya._sign_in_gate("codex", None), "name_required")


@screen("sync asks the writer on every run; a fresh writer answers without refreshing")
def _():
    reset()
    ya.apply_reader_bundle("codex", SLUG, ya.access_bundle(record("RT-LIVE", 10, 1)), "host-a")
    real_pull, real_refresh = ya.pull_access, ya.refresh_grant
    asked = []

    def pull(_h, _s, writer, _b):
        asked.append(writer)
        return ya.access_bundle(record("RT-LIVE-FRESH", 10, 0))

    def must_not_refresh(_rec):
        raise AssertionError("a reader triggered a refresh")

    ya.pull_access, ya.refresh_grant = pull, must_not_refresh
    try:
        out = ya.sync_lineage("codex", SLUG, b"")
    finally:
        ya.pull_access, ya.refresh_grant = real_pull, real_refresh
    assert out["state"] == "reader_pulled" and asked == ["host-a"], out


@screen("evidence never carries a token")
def _():
    reset()
    put_store(record("RT-SECRET-E", 10, 1))
    ev = ya.local_evidence("codex", SLUG)
    text = json.dumps(ev)
    assert "RT-SECRET-E" not in text and "eyJ" not in text, "evidence leaked a credential"
    assert ev["kind"] == "writer"


@screen("access is a machine verb: refuses to run without --for-reader")
def _():
    class A:
        for_reader = False
        harness = "codex"
        slug = SLUG

    expect_auth_error(lambda: ya.cmd_access(A()))


print()
print(f"{len(PASS)} passed, {len(FAIL)} failed")
sys.exit(0 if not FAIL else 1)
