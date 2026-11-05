# Working ledgers

This component provides local working context. Availability depends on the
installed generation. A ledger records revisable
working context and references to canonical tasks, reviews and evidence. An
actor's note is not admission, verified completion or standing doctrine.

The initial slice has one local authority and uses a private SQLite event
store under `~/.ygg/ledgers`. Existing approved root aliases remain usable;
this feature does not perform a physical root migration. Fleet transport,
replication, scheduler imports and garbage collection require their own
qualification. Never synchronize a live database, sessions or lock files.

Operations append attributable events. Replaying an operation ID with the same
input returns its prior result; reusing it with different input is held.
Concurrent payload changes require an expected revision. A stale proposal is
retained without replacing the winner. A note cannot certify itself by putting
`verified` or `done` in its payload. The current slice reports effective
verification as UNKNOWN. Human output is the default; `--json` selects the
machine result. Discovery and reads do not create a store or call a model.

The research basis is the swarm-harness2 working-ledger increment, including
the separation of a deterministic replay state from scoped observations at
query time. Hypotheses about reconstruction cost, missed obligations and false
completion remain unmeasured. Independent source review, fault conformance,
parser examples and an installed invocation must precede availability claims.

Sentence examples are checked against the real command owner:

```sh
ygg ledger create --title "Service upgrade"
ygg ledger ls
ygg ledger note --entry amber-otter-ab12 --text "Installed verification is open"
ygg ledger show --entry amber-otter-ab12
ygg ledger history --entry amber-otter-ab12 --json
ygg ledger resume
ygg ledger ops --pending
```

Use the actual entry slug or UUID returned by `create`. The local store has
its own stable identity; each entry is a named working context with its own
history and notes. Arbitrary task and job references can live in its payload.
There is no implicit active-context selection and no automatic task import.

Before retrying a lost acknowledgement, inspect `ops` and reuse the original
operation UUID with identical input. An automatic create key also determines
the entry identity, so a recovered request cannot create a second context.
Stale payload replacements are retained with a nonzero refusal result.

New stores use the logical `~/.ygg` root. An existing legacy store remains
readable, and relocation preserves identity. Two distinct default stores
require explicit selection. Existing empty, foreign, corrupt, shared or
public-readable files are held; a version marker alone never authorizes
schema repair. New files are 0600 before any content. Windows private-file
permission enforcement and fleet authority transport remain unqualified.


The initial storage protocol uses rollback journals. Reads inspect the SQLite
header first and hold WAL stores without creating coordination files. They
do not use an immutable view of a concurrently written database. An external
journal-mode change is outside this writer protocol and requires an explicit,
quiescent migration. Session labels and the full event including actor metadata
have byte bounds; replay keeps the original actor while its key binds the
semantic request. Numeric query bounds and revisions are validated before
queries or durable intents.
