# Session inventory and role ledgers

Status: owner-requested contract; implementation follows qualification of the
swarmharness2 core. These examples do not advertise installed commands.

The same inventory serves humans, shell pipelines, agents and optional GUI
clients. It uses registered fleet hosts and harness adapters, independent of
whether a desktop client is running.

## Sentence interface

```sh
ygg ls
ygg ls --metadata
ygg ls historic from 2026-10-01T00:00:00Z to 2026-11-01T00:00:00Z
ygg ls --harness codex from 2026-10-01T00:00:00Z to 2026-11-01T00:00:00Z
ygg ls historic --from 2026-10-01T00:00:00Z --to 2026-11-01T00:00:00Z --json
ygg ls historic --from 2026-10-01T00:00:00Z --to 2026-11-01T00:00:00Z --format tsv
ygg ledger
ygg ledger orchestrator
ygg ledger composer --job job-example --metadata
ygg ledger failures --json
```

`ls` without a window lists live sessions. `historic`, or either window
form, selects historical inventory. A harness filter narrows the adapters,
not the host set. Dates without a zone use the displayed operator time zone;
the response states the resolved UTC interval. Reject invalid or reversed
intervals. The interval is half-open `[from,to)`. Historical selection uses
observed session activity overlapping the interval, not creation time alone;
unknown lifetime boundaries remain explicitly unknown. A session created
earlier but active in the window must remain discoverable.

Default columns include host, harness, native session ID, state, last observed
activity and title. `--metadata` adds known model, role, campaign, job and
attempt IDs, parent/relay lineage, working directory and heartbeat age. Missing
values remain unknown. Metadata never includes credentials, raw environment
variables or full session bodies. Stable identity includes host and native
harness identity: recycled titles, row numbers or job names are not keys.

Tables are human readable by default, with headers, consistent columns and
control-character escaping. Redirected output has no ANSI escapes or terminal
width dependence; titles stay on one line and stable IDs are not truncated.
Explicit TSV provides escaped tab/newline fields and a fixed column order for
`sed`, `awk` and `grep`. Sorting and pagination are deterministic; continuation
tokens bind the filters and source watermarks so new activity cannot silently
skip records. A documented JSON envelope contains records, coverage, source
watermarks and continuation metadata; diagnostics go to stderr.

## Evidence and missingness

Each registered host and applicable harness adapter reports its coverage and
observation time. An unreachable host, unsupported harness, unreadable archive,
malformed source, stale heartbeat or uncertain process identity cannot become
an empty healthy result. Preserve available rows and report incomplete coverage
with exit 3. A fully covered empty selection exits 0. Distinguish live process,
idle process, active model turn, historical session and unknown state; a fresh
registry record alone proves none of the first three.

The role ledger joins immutable job and attempt identities with orchestrator,
sub-orchestrator, Composer, worker and independent acceptance events. Show
assigned work, planned/running/waiting/failed/accepted state, failure reason,
review status, repair action and evidence pointers. Our Composer combines
proposal and synthesis; the independent acceptance editor is a separate role.
A miner tick is not a completed Composer correction. A worker process exit is
not accepted work. Missing joins are visible debt, not invented success.

Historical failure lookup must preserve the exact host, attempt and native
session. Session inspection or attachment is a separate explicit action; listing
must not resume sessions, trigger model calls or mutate authentication.

## Qualification

Exercise every enrolled harness and host, both empty and populated selections;
offline hosts, partial permissions, absent/corrupt archives, recycled titles,
PID reuse, same native ID on different hosts, title control characters, Unicode,
time-zone and interval boundaries, long-lived sessions, concurrent pagination,
relay succession and retention gaps. Shell pipeline fixtures must preserve
untruncated IDs and parseable columns. Role-ledger controls must reject rc=0
without independent current-subject acceptance and show suspended review,
unclosed findings and failed repairs honestly. Finally compare installed output
to known real sessions and job evidence, rather than only adapter fixtures.
