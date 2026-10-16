# s56 — merge-key fingerprint (Q5 rank 1) + the Q4 falsifier matrix, yggterm campaign

- date: 2026-10-16
- seat: zcode (jojo) sess_d0f64053-0956-444a-96e2-e0c9cbc9ec43, sitting 56
- parent: 2026-10-16-yggterm-s54-projection-cache-sol.md (Q4 protocol + Q5
  ranking verbatim), board ACK-54b2eded50 (s55 outcome), plan ACK-b122eba32f
- code: lane/f7/merge-key-fingerprint 5dbcfd82 -> main 2f741bb39f7a (CI-deployed)

## What was asked (sol s54 Q5 rank 1, deferred from s55)

"per-input key costs + projection content fingerprint ... NEVER a bare
pointer as durable identity." The s54 profile measured the sidebar merge's
key hashing at ~7.6ms/window; both row-walk inputs re-hashed their whole
content on EVERY merge call (state.rs sidebar_merge_cache_key_parts):
stored_rows (browser.rows()) and stored_projection_rows (all_rows()).

## What landed (no new consult needed — the shape was fully determined by
the s54 verdicts; grinding law: low-level routine needs none)

1. Fingerprints computed where content is BORN, not per window:
   - projection: inside the OnceLock cold window (the one 19.6ms window
     already pays the walk; the hash folds in — no second cold window).
   - stored rows: inside rebuild_rows (verified this sitting: self.rows is
     written ONLY there — constructor + 6 state-change callers).
2. ONE hash definition, in core (write_stored_rows_merge_hash /
   write_projection_rows_merge_hash, re-exported at the crate root): the
   per-instance fingerprints and the key fn's fallback hash with the same
   function, so cached values are bit-identical to the slice-hash they
   replace — merge-cache semantics unchanged by construction.
3. Content identity, never pointer identity (sol's law): equal trees in two
   instances carry equal fingerprints (tested).
4. Fingerprint params sit directly after the slices they fingerprint, at
   every signature and call site; None = fallback slice-hash (crawl key,
   search-active fresh Vecs, plain telemetry variants stay on the fallback —
   sol Q4's search-active-separate law).
5. Dead sidebar_merge_cache_key removed (no callers; the parts fn is the
   only key builder).
6. Tests: fingerprint == fresh slice-hash across rebuilds; content identity
   across instances; projection fingerprint stable across filter rebuilds;
   locks 28/0, sidebar_merge 3/0, projection 12/0, snapshot 105/0,
   core 749/0, workspace --tests check clean.

## The Q4 falsifier matrix (protocol bars, from the s54 node verbatim)

Rig: tools/uxspeed/mergekey-rig.sh + mergekey_rig.py (this sitting rebuilt
the /tmp-ephemeral s54/s55 machinery as durable tools; dev Xvfb per-trial
display, hermetic YGGTERM_HOME/XDG_DATA_HOME, exe-proof per trial per the
[11.231] law; world = dev's real agent store ~20.3k sessions / 222 buckets
+ 400 rig-staged codex sessions).

MEASURED (control = pre-projection-cache build 6c2b112024ce~1 built in
/tmp/s56-control-target; candidate = deployed 2f741bb39f7a):

- CONTROL (6c2b112024ce~1, 3 trials, exe-proofed): 1303 search-inactive
  windows; snapshot.p.browser median 211.43ms (max 741.9) — the
  >=50ms control bar is MET, the matrix is conclusive. merge median
  24.39ms; key_ms (1936 recorded breakdowns) median 15.63ms max 57.4ms
  — the s54 ~7.6ms/window estimate was if anything an UNDERcount at
  this world size. 2022 warm builds (cache-miss counters).
- CANDIDATE (deployed main; served b52ee45c3089, which contains
  2f741bb39f7a; git diff 2f741bb39f7a..origin/main = 0 .rs files — the
  delta is memory-plane/docs only, so the fingerprint code measured is
  exactly what landed): 896 search-inactive windows; browser median
  1.27ms (p90 2.23 / p95 2.55 / p99 3.56); merge median 10.56ms;
  key_ms (195 breakdowns, only >=4ms merges record now) median
  0.025ms; 1470 warm builds; projection_init_count=1 in ALL THREE
  trials (the exact-one bar), init_counts control=None (the counter
  does not exist pre-cache, honestly reported).
- BARS: control-median>=50ms PASS (211.4). warm-median>=95%-below PASS
  (99.4% below). every-warm-window<=2.0ms FAIL AS STATED: 768/896
  windows meet it; the 128 hot windows are 2.0-6.8ms except THREE
  (34.0/36.4/76.4) which coincide with the webview edit-ack starvation
  episodes (85-100 "never acknowledged an edit batch" lines per trial
  gui log — a separate render-plane defect family, not the projection
  cache: init stayed 1 and only >=4ms merges recording means the cache
  was never the cost there). Per-call grain (sol warning): spiky
  windows (max > 3x mean) = control 0, candidate 1 — NO hidden
  single-call spike in warm windows.
- FIRST-SWITCH PAINT MATRIX: partially executed at DOM level —
  add/remove/swap/collision scenarios verified zero-mismatch on every
  DOM read that succeeded (candidate 6/20 scenario verdicts ok, control
  2/18), but at the 20k-session world the DOM-read leg is fragile on
  BOTH builds (webview ack starvation + virtualization). The >=100
  first-switch-paint zero-mismatch bar is NOT CLAIMED this sitting;
  the rig (now durable in tools/uxspeed) records every read and the
  next seat can finish the matrix on a smaller staged world or after
  the ack-plane fix. Search-active reported separately per sol Q4:
  the segment runs after the main legs and its windows are EXCLUDED
  from every bar above (analysis slices on search_t0_ms).

## Rig lessons paid for this sitting (for the next seat)

1. The daemon's FIRST cold local_tree_scan on dev's 20k-session store took
   108s once (page cache made later runs ~2s) — trials before the scan
   measure a tree that does not exist. Gate on TWO local_tree_scan perf
   events with total_sessions>0. NEVER gate on describe's row_count: it is
   the VISIBLE-row count (a collapsed 20k tree reports ~7), not the world.
2. Expanding a 20k-session tree flat starves the webview's edit-ack plane
   (unacked batches, "releasing the flush gate" errors, dark DOM windows).
   Bounded expansion (<=14 clicks/round, live region + a few groups) is the
   workable posture; full expand-all is NOT a valid rig state at this world
   size. (This is itself an F7-grade observation: the ack plane degrades
   non-fatally but the DOM silently lags — see smoke7 gui log.)
3. DOM reads need frame freshness: `server app state`'s root_render_count
   must advance past the pre-action baseline before a paint verdict read,
   else the stale-frame flake (smoke8's remove scenario) reads as a product
   bug.
4. `server app state` (DescribeState) is the verb that carries describe's
   browser/metrics payload — `server app describe` is not a verb; Probe's
   plain "describe" call in tools/uxspeed/uxprobe.py therefore never found
   the counter there.

## What remains (Q5 rank 2-3)

- The 500ms TTL audit (busy-hint expiry depends on it — never remove
  without a proven replacement freshness mechanism).
- Reprofile the residual merge: control 24.8ms -> candidate 13.4ms median
  window means on this world; what remains inside the merge (the clone of
  the cached Vec on return, enrich_sidebar_rows_with_live_titles, the
  render-side fold) is the next decomposition.
- The stored_rows .to_vec() at the snapshot site (search-inactive arm)
  is recorded for that unit, not fixed blind here.
