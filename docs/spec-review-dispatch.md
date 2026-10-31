# Trusted worker review dispatch

Status: proposed bridge design. The accepted `session_review` and
`review_completion` kernels are prerequisites, not a deployed dispatcher.
Current queue results remain review-pending until this bridge is qualified.
Our Composer proposes/edits/composes improvements; the configured independent
acceptance reviewer remains the separate `roles.editor` execution.

## Admission and scope

A trusted task adapter captures the original request bytes/hash, criteria,
work/milestone identity, parent obligation, source scope, worker identity,
physical dispatch UUID, owner-stop generation and activated policy revision.
This record lives outside the worker workspace. A worker RESULT is evidence;
it cannot create admission, replace criteria or confer authority. Old tasks
without a trustworthy admission remain visible and held for reconciliation.
No inferred timestamp winner or automatic conversion from an old completed
claim creates acceptance.

A bounded source milestone may finish while its parent repair remains open.
If the admitted criteria require installed behavior, source approval alone
cannot finish that task. Deployment and affected-path verification must be
explicit admitted milestones or evidence obligations before parent closure.
The reviewer sees the original request as well as the proposed criteria, so
an agent's decomposition cannot silently remove the owner's requirements.

The portable Admission binds its explicit fields. The controller envelope
additionally seals source scope, original request reference and parent/milestone
boundaries. Those envelope fields are immutable and hashed into the durable
job manifest; trusted adapters validate them before constructing kernel inputs.
This is a trusted-controller boundary, not isolation against a malicious process
with the controller's operating-system identity or access to its signing key.

## Durable job and attempt state

A job identity covers admission identity, a content artifact identified by
logical scope/relative names/byte hashes, selected review policy and protocol
version. It is independent of a mutable workspace name, retry ordinal, host
clock or CLI role label. The review ID is derived
from that job identity, preventing accidental reuse of an unrelated chain.
Duplicate admission of identical bytes is idempotent; contradictory bytes for
one identity HOLD and preserve both diagnostic inputs. The kernel Job's digest
and SessionReview subject digest are local execution bindings, not this global
content identity: the current review kernel includes materialized file paths.
Record that mapping explicitly. Physical-root migration and cross-host replay
of those bindings remain held until their separate relocation adapter passes;
copying a local digest does not qualify another materialization.

The job manifest and append-only event journal use owned private storage,
no-follow path checks, one stable mutation fence and durable publication.
Unknown schemas, partial journal records and unavailable storage HOLD before
launch. No collector deletes active jobs, their subjects or native invocation
artifacts. Model calls and network operations happen outside local locks.

States distinguish waiting, leased, staged, invocation-started, review-pending,
accepted-subject, needs-repair, suspended and task-applied. Accepted-subject is
not task-applied. Each attempt has a fresh immutable identity and lease token.
Every mutation compares the expected job revision and token. Lease timeout
alone cannot prove a model call was never submitted or completed. An expired
lease after invocation-started is unresolved/suspended until the native adapter
reconciles the invocation; it never silently launches a duplicate. Pre-launch
claims are recoverable only under the declared serialized recovery contract.
Owner stop fences claims, launch, verdict intake and task application.

Before the first external submission, durably commit an invocation-intent event
with its attempt/token. Submission authorization is a second durable transition,
serialized on the SAME fence as owner stop: compare the stop generation and
attempt/token, then commit `submission-authorized`. This commit is the logical
launch linearization point. If stop commits first, authorization fails and no
transport call is permitted. If authorization commits first, this attempt is
already logically in flight; stop cancels it where supported, even if network
emission has not yet begun. It cannot guarantee that an already-authorized
external request emits no bytes. An adapter may use a broker held behind a
local start barrier until authorization commits and the fence releases. It must
not make network/model calls while holding the fence. A post-stop attempt needs
new authorization after an explicit authorized resume.

A crash after invocation intent, including before authorization, is unresolved
unless the serialized journal and native broker prove it could not have
submitted. A crash after authorization, including before a result/process
receipt, means the request may have been submitted. Absence of a started receipt
is not proof of no call. Recovery requires authoritative reconciliation or stays
suspended; lease expiry cannot authorize a duplicate. Stop prevents verdict
application and new calls and records unresolved remote execution explicitly.

## Stage and invoke

Stage declared files or immutable Git blobs within the sealed scope. Reject
path escapes, unexpected symlinks, unknown kinds and changed scope. Record the
original source revision/location, byte hashes and applicable check receipts.
A content-addressed private snapshot and read-only source pin protect the
review from concurrent builds or workspace changes. The SessionReview subject
binds the exact staged files; the source/artifact mapping remains inspectable.
Existing mutable `target/release` paths are not publication identities.

Load the activated configuration through the kernel, resolve `roles.editor`
and its admitted default or scenario-authorized fallback, and bind the exact
configuration and binding digests. Containment, deny latches, authority scope,
capabilities, budgets and owner stop constrain every execution attempt. A quota
refusal cannot select an unadmitted model, change funding routes, consume a
banked reset or become success. Record deferred review and the next eligible
retry; preserve the complete history and configured swap limits.

Before external invocation, require the applicable privacy result for the
exact bytes and destination. Covered unknown, missing, incomplete, stale or
negative receipts HOLD; validate again immediately before launch. Registered
coverage is explicit. Do not send raw prohibited content to a classifier to
ask whether it is prohibited. A shell command's failure cannot be ignored by
a following launcher. Unchanged approved subjects can reuse evidence only
through a separately qualified deterministic exemption path; the initial
full/lightweight adapter holds unsupported exemptions.

The native adapter records actual harness, model, effort, account/plan binding
references, native session UUID, process start identity, submission state and
result artifacts. It verifies the configured execution rather than trusting
worker-authored metadata or a footer. Editor and worker native identities are
distinct; different role labels do not establish independence. Credentials
remain in the local authorized native/vault channel. Recover access through
the configured credential chain before escalating to the owner; distinguish
blocked recovery from quota, reserves, policy holds and missing work.

A trusted invocation record plus exact payload/chain-head verification admits
the verdict into SessionReview. The worker cannot supply its own verifier or
signer. Quota, timeout, transport loss, partial output, missing provenance or
storage failure retain the attempt and review debt. CLI exit zero does not
establish a completed native turn or accepted work.

## Resolve, repair and apply

Revalidate the complete review chain, original admission, actual current
subject, configured editor policy, checks, criterion-owned debts and every
finding's editor closure. Require canonical COMPLETE and the final review
head. The accepted kernel produces eligibility and exact CAS expectations;
it is not a permission token or a queue mutation.

The applying adapter obtains current observations independently of worker
output and atomically compares task state, physical dispatch UUID, original
prompt/admission, owner-stop generation, review-job revision/token and review
head. Source publication uses an immutable subject or its declared common
publication fence. A mutable-file rehash before an unrelated database write
cannot by itself close a filesystem/queue race. Changed or unavailable inputs
HOLD without overwriting cancellation, newer work or another actor's result.
Only the matching admitted milestone can move to applied/completed.

REVISE retains the chain and creates a repair continuation through the existing
trusted packet/permission gate. A fresh worker receives original criteria,
findings, exact subject and full history; its assertion cannot close a finding.

Succession creates a NEW physical dispatch, actual worker native identity,
admission, job and review chain. Never rewrite an old review opening to name
the successor or reuse its acceptance for a different worker. The sealed
successor envelope binds the root request/parent obligation, predecessor job
and authenticated review head, complete unresolved finding references, unchanged
criterion definitions and permitted source scope. Admission.prompt_sha256 is
the actual queued instruction body for this attempt; a derived repair brief
has a new hash, while the original owner request retains its separate binding.

A qualified succession bridge authenticates predecessor findings and obtains
explicit authenticated closure of each inherited reference by the successor's
independent reviewer, binding predecessor head/finding/criterion to current
subject and review head. A fresh chain's empty findings never close inherited
debt. The new editor must differ from every contributing worker native identity
in that obligation's lineage. Criterion-owned deferral rules remain unchanged.
Append an evidence edge to the work ledger; retain the old REVISE subject and
history as failed/superseded evidence rather than repainting it COMPLETE.
SessionReview has no worker-rebinding or implicit finding-import authority.
Until this explicit bridge and its controls are implemented, automatic successor
dispatch/application HOLDs with the work and inherited findings retained. The
initial dispatcher cannot claim a general repair loop from one source approval.

Accepted work emits a bounded evidence/outcome record for the experience library.
A variant or rubric is not credited as an improvement without its declared
prospective/held-out outcome comparison. Missing automation stays visible.

## Falsifiers and rollout

Test forged or stale worker approval, self-review under a different role name,
missing privacy or provenance, current-subject/check/policy drift, owner stop
at every boundary, changed dispatch under a reused ordinal, malformed storage,
concurrent claims, partial durable writes, launch-before-receipt crashes,
quota/storage suspensions, late results from old attempts and unavailable native
sessions. Repeated drain must not duplicate model calls or task effects.
Race stop against submission authorization with deterministic barriers: stop
winning yields zero calls; authorization winning yields one tracked in-flight
attempt with cancellation/ambiguity, never application after stop. Kill before
and after intent, authorization and broker release, then restart drain. For
succession, change worker/dispatch while retaining the request: old opening or
receipt reuse and missing inherited debt HOLD. Verify inherited closures and
actual independent native identities before a successor can apply.

First qualify storage/leases, then staging/activated-policy resolution, the
native invocation adapter, and finally the queue CAS hook. Enable one admitted
real repair with a known failing control, source/editor acceptance, controlled
application and an affected-path reprobe. Retain parent obligations and inspect
actual job/session/artifact identifiers. Only after that trial extend the
controller to other native harnesses and the Composer proposal intake.
The trial must show the original failure and negative controls, not merely
that the transport or timer ran. Whole yggterm 4.0 readiness remains separate.
