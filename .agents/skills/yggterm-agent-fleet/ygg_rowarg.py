"""How a row is NAMED on the fleet's command line — the one owner of that question.

⛔⛔ **THE DEFECT THIS EXISTS TO END: naming one row took three spellings, and
which one was correct depended on the verb.** Measured 2026-08-20 while reaping
two finished lanes:

    ygg-monitor.py subscribe   --uuid <u>     # flag
    ygg-monitor.py unsubscribe        <u>     # positional — the SAME tool
    ygg-booter.py  unsubscribe --row  <u>     # a third flag, a third tool

Every one of those refuses the other two with an argparse usage dump, which
reads as *"this row is not subscribed"* to anyone scanning output. So the
natural guess is wrong twice out of three, and it is wrong at the exact moment
a caller is tidying up after a finished lane — the moment where a silently
skipped unsubscribe leaves a corpse being nudged forever.

⚠ **Why it survived so long: each spelling is locally reasonable.** The monitor
grew subparsers, so a positional was natural per-verb; the booter has one flat
parser over all its actions, so a flag was natural there. Nothing is wrong
inside either file. The defect only exists in the space BETWEEN them, which is
exactly the space no single file's review covers.

⭐ **The rule this module encodes: every verb on both watchdogs accepts a row
named either way — positionally or by flag — and `--uuid` and `--row` are the
same argument.** A caller should never have to remember which verb they are
talking to. The value may be a bare uuid or any addressable row path
(`scheme://host/<uuid>`); the last path segment is the uuid, exactly as the
claim script has always taken it.
"""


def bare_uuid(value):
    """The uuid inside a row name, whatever shape the caller used.

    ⛔ `$YGGTERM_SESSION_ID` IS NOT A BARE UUID — it is `cc-runtime://<uuid>`.
    Used as a filename the `//` becomes a path separator, so a subscribe died
    with FileNotFoundError on `.../monitor/cc-runtime:/<uuid>.json`, and only
    the flag spelling worked. Take the last path segment, as the claim does.
    """
    return (value or "").strip().rstrip("/").rsplit("/", 1)[-1].strip()


def add_row_argument(parser, *, dest="uuid", positional=True, required=False):
    """Declare the row argument in every spelling, on any parser.

    `dest` is the attribute the tool already reads (`uuid` on the monitor,
    `row` on the booter) so this changes the SURFACE without moving anyone's
    source of truth. The flags land in a separate dest and are folded in by
    `resolve_row`, which is what lets a positional and a flag coexist.
    """
    if positional:
        parser.add_argument(dest, nargs="?", default="",
                            help="the row: a bare uuid, or an addressable "
                                 "`scheme://host/<uuid>` path")
    parser.add_argument("--row", "--uuid", dest="_row_flag", default="", metavar="ROW",
                        help="the same row, named by flag instead of position — "
                             "both spellings work on every verb")
    parser.set_defaults(_row_dest=dest, _row_required=required)


def resolve_row(args, *, env_fallback=""):
    """Fold the two spellings into one value, and REFUSE a disagreement.

    ⛔ Two different rows named in one call is never a preference to resolve —
    it is a caller who believes they are addressing something they are not.
    Picking either one silently would act on a row nobody asked for, which is
    the failure this whole plane exists to prevent. Raises ValueError.
    """
    dest = getattr(args, "_row_dest", "uuid")
    positional = bare_uuid(getattr(args, dest, "") or "")
    flag = bare_uuid(getattr(args, "_row_flag", "") or "")
    if positional and flag and positional != flag:
        raise ValueError(
            f"two different rows named in one call: {positional[:8]} "
            f"(positionally) and {flag[:8]} (by flag). Name one row.")
    value = positional or flag or bare_uuid(env_fallback)
    setattr(args, dest, value)
    return value


def row_session_id(row):
    """The id of a row **we are holding the JSON for** — its own answer, not ours.

    ⛔⛔ **THE DEFECT THIS EXISTS TO END: the fleet re-derived a row's identity by
    slicing its path, and that is correct only when the path happens to END in the
    id.** Measured 2026-08-22 against the live row plane: of **281 session rows,
    246 (87.5%) derived an id that is not the session's id**, and five rows of one
    CLI collapsed onto a single value because their store spells identity as a
    directory and names every file the same thing:

        full_path  …/brain/<a>/.system_generated/logs/transcript_full.jsonl
        full_path  …/brain/<b>/.system_generated/logs/transcript_full.jsonl
        derived    'transcript_full.jsonl'   ← for BOTH, and for three more

    Only a row addressed by a `scheme://host/<uuid>` path derived correctly, which
    is why this survived: the rows a lane touches most are live ones, and live rows
    wear the scheme. Every row at REST is path-shaped, and so is every row of a CLI
    whose sessions are files rather than sockets.

    ⇒ **What made it invisible is that `full_path` is not one format.** It is a row
    ADDRESS, and its shape is a property of the CLI and of whether the session is
    live — so a slice that is right in front of you is wrong four times out of five
    a little further down the tree.

    ⚖ **The id was never missing.** yggterm resolves it through the CLI registry,
    which knows that one store names the session's DIRECTORY and another its FILE
    STEM, and it publishes the answer on every row as `session_id` — populated on
    **281 of 281** rows measured. So this is not a gap to fill, it is a second
    encoding to delete: the registry already decided, and the fleet disagreed with
    it in nine files.

    ⛔ Use this whenever a row's JSON is in hand. `bare_uuid` is for the OTHER
    question — what a human typed on a command line — where there is no row to ask
    and the last segment is the best available guess.
    """
    if not isinstance(row, dict):
        raise TypeError(f"row_session_id needs a row's JSON, got {type(row).__name__}")
    published = (row.get("session_id") or "").strip()
    if published:
        return published
    # ⚠ Only for a row that carries no id at all — a folder, a group, a machine.
    # Falling back is right here (the caller wants SOME stable key) and wrong for
    # a session, which is why the emptiness is what selects it rather than a flag.
    return bare_uuid(row.get("full_path") or row.get("path") or "")
