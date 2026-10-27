# Heartbeat installation

This is one deterministic, observation-only service with internal 15/60/300s
cadences. Install on the fleet server; no timer unit is used. A laptop may
run discovery and alarms read-only. This controller cannot execute idle,
burn, enqueue, repair, change auth or activate policy. Idle admission is
separately enforced by the client.

From the reviewed checkout, copy `heartbeat.py` and `planes.json` to
`~/.yggterm/bin/heartbeat/heartbeat.py` and `~/.yggterm/config/planes.json`.
Copy `../task-ledger/{plan.py,booking_outbox.py}` into the adjacent
`~/.yggterm/bin/task-ledger/` directory (the controller imports their pure
parsers). Copy `ygg-heartbeat.service` to `~/.config/systemd/user/`, then
run `systemctl --user daemon-reload` and
`systemctl --user enable --now ygg-heartbeat.service` on the server.
Installation and enablement are separate from source acceptance.

Review reader hosts and commands first. Each reader is bounded at 10s;
independent readers run concurrently, and shared usage reads are deduplicated.
The healthy-path discovery bound is 60s plus one reader deadline, with the
finding persisted on that urgent pass. A hung or failing CLI yields
unavailable, never zero. Current usage includes a runtime telemetry read
that can exceed this deadline; this intentionally remains unavailable until
the existing reader completes within budget. No new credential reader is added.

Register watcher `id`, `health_path`, `max_age_s` (default 45), and optional
task id in `watchers`. Health files carry `as_of`, `pid`, `boot_id`; absent
boot evidence is unknown. Claimed tasks without a live registered watcher
are findings, not claims that the task has failed. Monitor conflict paths
can be registered explicitly. Empty source alone is informational.

Outer plane quality is verified/unavailable/conflicting. A start-plan
observation with unreadable balance carries `obs.quality=verified-metadata`
and `remaining=null`. Free-tier route health lacks an authoritative quota
reader and is explicitly unavailable. No funding pools are summed.

Three consecutive observations of a fault create one retained alarm;
three clear observations create one recovery. Cheap passes never count a
cached urgent/full result as another observation. Notifications survive
restart. Board mutation uses the existing `msgboard post` verb, with a
transition id and ACK receipt. Configure `board_cmd` to the registered
privacy-gated publisher where required by enrollment. Only fixed finding
codes, transition ids and the controller UUID leave this process. Failed
delivery is retained and logged to stderr/journal. Retries are safe because
idempotency lives AT THE SINK (keyed sink, 2026-10-28): every post carries the
retained transition id as `--idempotency-key`; the msgboard sink serializes
keyed append + readback, returns the retained ACK for a same-key/same-body
retry, and refuses a conflicting payload. There is NO unkeyed fallback: an
installed publisher that predates `--idempotency-key` holds the alarm pending
until the canonical tool is deployed. This does not claim exactly-once
transport through an uncooperative remote publisher, independent observer
coverage, or deployment acceptance.

The outbox replayer is the separate explicit command
`python3 ../task-ledger/booking_outbox.py replay --once`; this observation
service counts retained booking debt and does not mutate hub doors.
