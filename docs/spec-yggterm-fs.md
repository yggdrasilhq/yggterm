<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Filesystem specification moved to the Ygg operating layer

The current contract is [spec-ygg-fs.md](spec-ygg-fs.md). Its canonical
logical root is `~/.ygg`, with a phased compatibility transition from
`~/.yggterm`. The first phase creates a new-name alias to the existing store;
it does not move active data, rewrite policy, or claim completed subtree
conversion or garbage collection.

The previous document described a target layout, not an inventory proving
that each owner had migrated. Use `ygg fs status` and `ygg fs path <id>` to
see selected identity and current/target placement. ynpm owns executable
generations and registration; applications own their mutable state.
