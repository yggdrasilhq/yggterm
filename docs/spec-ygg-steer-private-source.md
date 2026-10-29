<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Private source compilation and legacy projection import

Status: read-only compiler increment; migration and activation are not
implemented by this module. It supports the portable steering contract's
later private source import. A successful compile is a proposed projection,
not accepted policy, target publication or repository retirement.

The private manifest has schema/compiler version 1 and a target table. Each
harness declares a canonical relative target, an ordered list of sealed
source parts, and foreign-owner regions to preserve from an exact live
snapshot. A part binds its relative path, SHA-256 and literal/guarded mode.
No template code, shell command or environment interpolation is executed.

The caller supplies the canonical directory of an enrolled private source
revision. Intermediate source-directory and leaf symlinks are refused.
Sources must be owned regular files with current matching digests. Every
part is bounded to 4 MiB and the complete source/live projection inputs to
8 MiB. Unknown schema fields, modes, target collisions, path traversal,
duplicate guard ownership and malformed foreign regions HOLD.

Literal parts preserve owner-authored historical text as opaque content.
They may contain annotated historical comment delimiters; these are not
new tool-owned template slots. Guarded parts receive one explicit wrapper
and may not contain its own delimiter. A source part cannot claim a region
registered to another owner. Complete foreign regions retain their exact
internal bytes, including CRLF. Missing required or ambiguous foreign
regions HOLD. Surrounding target text is not automatically adopted or
discarded: the full old/new projection is a reviewable proposal. Future
activation must explicitly resolve all live drift and user-owned text.

Output binds the manifest, every source digest, input snapshot and proposed
output. It always declares proposal-only authority and requires private
projection review. It neither selects an active revision nor writes a
harness file. The separate activation/stamping transaction must enforce
current authority, expected parent, target identity, cooperative writer
fences, foreign/user-byte preservation, durability and actual readback.

Legacy import first preserves repository history and all working files in
private recovery storage, then derives a private manifest from the known
old assembly. Compare every resulting projection against the old compiler
and every actual harness target. Source equivalence does not erase an
existing live drift. Existing historical evidence retains its original
paths/digests. Public source and invented fixtures contain no private policy.

Required controls include altered source bytes, malformed/duplicate foreign
regions, CRLF, historical literal comments, normalized target collisions,
unsafe/symlinked paths, unsupported schema/mode, wrapper injection, bounded
inputs and zero target mutation. Full fleet import, private source editing,
orphan skill relocation, policy promotion, watchdog replacement and eventual
legacy deletion remain separately recorded obligations.
