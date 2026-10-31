# Credential recovery contract for operator policy

This is a sanitized onboarding example, not an enrolled credential source.
The operator's private policy supplies service identity, vault entry references,
allowed recovery mechanisms and any human-only factors. Secret values, account
identities and private endpoints never belong in this public seed. A fresh
installation must report an absent route explicitly; this document cannot
manufacture account access or consent.

Before asking the owner to repair access:

1. Inspect the native client's status and configured service identity. An
   endpoint responding is different from authenticated access.
2. Find the configured recovery protocol through registered infrastructure/data
   skills and private operator policy. Verify that the referenced door exists;
   a stale or missing pointer is a steering defect.
3. When authorized vault-backed recovery is available, resolve its named entry
   within the credential process and perform the supported native login or
   refresh. Pass values through stdin or the native credential channel; keep
   them out of argv, transcripts, source, classifier inputs and diagnostics.
   Do not copy native keyring, session or auth-profile files between hosts.
4. Recheck native status and the requested operation. Keep failed attempts and
   unknown results visible; a login acknowledgement alone is not task success.
5. Ask for the necessary human recovery only when both service access and its
   configured authorized recovery are blocked, such as a locked vault or a
   human-only factor that the configured protocol cannot satisfy. Name the
   measured failed stages and required intervention without secret values.

A healthy session avoids forced login. A rejected secret is not permission to
reset accounts, weaken authentication, buy quota or replay an uncertain request.
Unknown quota, a protected reserve, missing work or a policy hold remains its
own state after authentication succeeds.

Treat premature escalation as a Composer regression case: preserve the actual
failed retrieval and counterexample, propose a narrow pointer/policy correction,
obtain independent acceptance of the exact change, apply it under the owning
writer's rules, and test fresh sessions on held-out cases. Useful controls are
healthy service access, expired service with available authorized vault, both
paths blocked, missing/stale recovery door, and a quota/reserve hold with healthy
auth. Count success only after affected behavior and counterexamples are
verified; a source patch, model agreement or timer tick is not automatic repair.

The existing initialized seed and private policy are not silently replaced by
shipping this example. Seed selection and private-authority migration have
separate reviewed plans and consumer/readback obligations.
