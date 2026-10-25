"""swarm-harness kernel (portable): contracts, config, ledger, policy,
receipts, registry, editorial selection, empire index, maintenance boundary.
Infra-specific code (queue, timers, hosts, credentials) lives in adapters
OUTSIDE this package — owner directive 2026-10-25: swarm-harness changes
start FROM this seed, never as bolt-on addons."""

__all__ = [
    "swarmconfig",      # v2 schema, validator, activation store, deny latches
    "swarm_ledger",     # multi-directional ledger: capture/adjudicate/journal
    "empire_index",     # P1 bounded consult view (v2: freshness + registry)
    "registry",         # P1 v2 campaign registry + procedure library
    "editorial",        # editor hot-swap scenarios + selection receipts
    "legacy_supply",    # one-way floors.json generator (phase-5 cutover)
    "maintenance",      # deterministic maintenance boundary (controller/replica)
    "executor_adapter", # queue transport gate (installed command, never YAML)
]
