# Repository completion

Apply this default when no explicit owner or repository instruction overrides
it. Establish ownership from the operator's instructions and repository
configuration; push access alone does not establish ownership.

- **Operator-owned repository:** edit in a lane worktree, obtain the applicable
  independent acceptance and checks, integrate through the normal CI workflow,
  then push the default branch. A pull request is optional unless repository
  policy requires one.
- **External contribution:** push the contribution branch and submit a pull
  request against the upstream repository. Do not push its default branch.
- **Unestablished ownership:** retain the work and record the publication
  question. Do not infer authority from a token's permissions.

For owned repositories, completion includes all of the following:

1. Verify the remote default branch with `git ls-remote`. It must contain the
   reviewed change. A successful local merge or push acknowledgement is not
   sufficient. Record any explicit deferral and its remaining obligations.
2. Classify each lane using ancestry and, where necessary, exact patch and
   current-tree evidence. A commit count, similar text, or a worker's report
   cannot establish that a change landed. Never merge an old branch solely to
   make it appear landed: it may restore obsolete behavior.
3. Before reclaiming a worktree, verify it is clean, contains no unpreserved
   ignored work, and is unused by running processes, active sessions, queue
   leases, CI subscriptions and owner pins. Incomplete visibility means retain
   it. Preserve the review subject and evidence needed for audit or replay.
4. Remove only qualified worktrees with `git worktree remove`, without force.
   Delete their merged local branches. Delete the matching remote lane only
   after rechecking its exact tip and proving it is still integrated and unused.
   Use an expected-tip lease for remote deletion; a collaborator's newer push
   invalidates the cleanup decision.
5. Read back the remote refs and local worktree list. Report retained lanes
   with the reason and remaining owner. Never erase dirty, occupied or
   unmerged work to improve the count.

Repository integration and runtime delivery are separate facts. Merged source
does not prove installation, a live session attachment, a usable GUI, or an
operating correction loop. Record those qualifications separately.
