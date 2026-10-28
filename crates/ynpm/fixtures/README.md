# ynpm release fixtures

`release-manifest.example.json` is the **shape reference** for the
schema-1 GitHub release manifest that `crates/ynpm/src/release_contract.rs`
validates and `docs/ynpm-desktop-releases.md` specifies. The release/CI
pipeline (owned by the parent integration lane) generates the real
`release-manifest.json` asset per release.

**The all-zero sha256 digests are placeholders.** This file is a shape
example for humans and tooling; it is never an install input. The zero
digests pass structural validation (64 hex characters) but match no real
archive, so installing from them fails the mandatory checksum gate - which
is exactly what a placeholder should do.

Everything else in the example is invented for the public repo: package
ids, filenames, and URLs are synthetic. No real release asset or digest
belongs in this tree.
