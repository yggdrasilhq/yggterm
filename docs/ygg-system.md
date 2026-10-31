# The yggterm operating boundary

`ygg` is the primary porcelain for a swarm operating layer distributed over
conventional operating systems. The GUI, a web client and a mobile client
are interface choices. Durable state, policy, execution ownership and work
identity belong to the system. Existing focused tools remain plumbing under
the command router while native packaging is qualified.

Research hypotheses, mathematical assumptions and executable falsifiers
precede engineering claims. Swarmharness2 defines the architecture; this
repository implements and qualifies its contracts. Neither a paper, a source
patch nor a release label certifies the running system.

## Taxonomy

| Term | System meaning |
|---|---|
| Dreamer / AgentWorker | A bounded execution role on admitted work, with retained attempts and independent acceptance obligations |
| Group of affairs | Related work and dreamers; independent of a GUI's arrangement of rows |
| Work / task | A source-owned obligation or admitted execution unit with its own criteria; separate from its executions and interface position |
| Execution attempt | One physical try at admitted work, with a dispatch identity, route, observations and outcome; an ordinal can be reused without making it the same attempt |
| Execution row | An addressable execution seat or daemon-owned terminal address; a GUI row is a projection of that address, not a native conversation ID |
| Native harness session | The model harness's conversation identity and context/history; join it explicitly to observable rows and attempts using host and harness identity |
| Dream orchestrator / sub-orchestrator | Coordination, admission, allocation and supervision within an explicit policy and scope |
| Relay | Context succession on the same work and history; an ordinary retry is a different lifecycle |
| Composer | Evidence mining, candidate editing, probing, selective composition and experience formation; it combines Harness²'s candidate-editor and composer responsibilities |
| Independent acceptance reviewer | The configured external check of the exact current worker subject and criteria; the deployed schema still resolves it through `roles.editor` |
| Experience library | Versioned procedures and variants tied to evidence and prospective outcomes; a memory rollup alone does not qualify a reusable improvement |

The deterministic observation miner is one Composer component. Its running
timer does not establish that admitted repair execution, affected-path
verification or learned variants work. Similarly, a queue's `review_pending`,
`retained_partial` or `submitted` state preserves an unfinished obligation;
a process exit or remote ticket is not work acceptance. The independent
reviewer remains separate from the worker and its Composer functions.
Rows and sessions are not one-to-one: multiple addresses can expose one
conversation, and an execution can reconnect to a session or create a fresh
one. A relay creates context succession while retaining the work's history;
an ordinary retry does not imply that succession. These are architectural
identity rules; the session/role-ledger implementation remains separately
qualified.

## State and interfaces

The logical root is `~/.ygg`, with versioned ownership and compatibility
aliases described by the filesystem contract. Public steering mechanisms and
sanitized seeds live in this repository; actual private policy belongs to
operator state. Legacy authority is retired only after qualified migration
and fleet readback. Important information and useful app caches require their
owner's synchronization adapter. Live databases, lock identity, credentials
and process-local resources are not a recursive directory-copy protocol.
Ychrome is a libyggterm application using WebKitGTK; its state contract must
follow its own owners, not a Chromium profile recipe.

See [filesystem](spec-ygg-fs.md), [steering](spec-ygg-steer.md),
[synchronization](spec-ygg-sync.md), [inspection](swarm-introspection.md) and
[session/role-ledger sentences](spec-session-and-role-ledgers.md). Those
contracts retain planned or unproved links explicitly. Current qualification
and install limits live in [the component status](operating-python-status.md).
