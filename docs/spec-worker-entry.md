# Worker entry preconditions

Status: bounded launch guard; task authority, filesystem confinement and full
native review dispatch remain separate qualification gates.

A memory precondition written in a worker prompt is advisory. The trusted
launcher must execute and validate it before constructing the model harness.
Let `G` mean the exact admitted harness/namespace gate passed, `S` mean the
qualified gate source identity matched and `L` mean the harness was started:

`L implies G and S`; therefore `not G or not S implies zero harness starts`.

The gate uses explicit task routing rather than guessing the namespace from
the working directory. A successful process exit alone is insufficient: the
bounded JSON result must identify the requested harness and namespace and
report known zero conflicts and missing objects. Boolean zero, malformed data,
unknown schemas, timeouts, output overflow and unavailable storage are held.
No shell or pipeline can turn an inner failed gate into a successful launch.

The source is an owned, qualified Python entrypoint whose exact content hash
is supplied by the trusted installation/admission adapter. The initial Linux
guard executes an opened source descriptor, retaining its identity across an
ordinary atomic deployment replacement. Its interpreter uses isolated mode,
excluding caller Python paths and user site hooks. It does not attest the interpreter,
all imported dependencies or hostile in-place modification by the same UID.
This helper exclusively waits for its gate child. It observes exit with
`waitid(WNOWAIT)` and reserves the unreaped leader PID until bounded TERM/KILL
cleanup of the owned gate process group finishes. Independently detached
sessions and privilege-changing gate children are outside that group contract.
The initial local procfs qualification observes same-UID group members until
none is executing; unknown visibility or an unconfirmed cleanup remains held
with the gate process retained in memory. It is not a cgroup confinement claim.
[Linux wait semantics](https://man7.org/linux/man-pages/man2/waitpid.2.html).

Every attempt retains private gate evidence. Failure produces a typed hold and
never calls the harness constructor. A successful receipt is an observation,
not an execution permission, task completion, editorial approval or reusable
authorization token. The admitted controller must still enforce policy, quota,
budgets, privacy, durable intent, owner stop and replay rules at their proper
boundaries. This primitive does not implement retry or queue completion.
Command/environment inputs are copied before gate I/O. Diagnostic storage
failure cannot mask the typed in-memory hold; a post-spawn hold retains the
process handle and marks that the harness started, with its native effect
unknown and no retry authorization.

Passing an entry gate also does not authorize a worker to repair a subsequent
conflict outside its task. Such a conflict retains work and returns to the
controller. Tool and filesystem capability enforcement is a separate required
boundary; a prompt or same-user cooperative guard is not an OS sandbox.
Unrelated ledger resolution, policy changes or claimed approvals cannot be
manufactured from a failed precondition.

Falsifiers use invented local gate scripts and short-lived child commands:
failed/false/mismatched/unknown gate results, changed source, timeout and output
overflow must create no child marker; valid evidence starts exactly the
requested command. Repeated explicit calls are separate attempts, not a safe
retry protocol. Linux observations do not qualify Windows or macOS behavior.
