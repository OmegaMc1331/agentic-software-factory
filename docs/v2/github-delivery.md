# GitHub inspection and controlled delivery

P12 reuses `factory-github` and the locally authenticated `gh` CLI. It does not create another credential store or authentication provider. Repository identity starts with the configured owning Git remote and is verified against authenticated GitHub metadata/ID. A remote URL alone is not proof of access. PR base/head repositories remain distinct, including forks.

## Reads

Explicit bounded reads provide repository metadata, paged Pull Requests, files, reviews, comments, check runs and commit statuses. Unknown mergeability remains unknown. Pending/running/success/failure/cancelled/skipped/unavailable are not collapsed into Passed. A denied check endpoint remains unavailable even if other PR data is readable. Rate-limit/auth/network errors are classified and do not launch polling/retry loops. API pages and observed timestamps are visible; bounded data is not presented as a complete remote history.

## Preview and publication

The delivery preview reuses the existing evidence-derived PR body and eligibility checks, shows actual integration files/commit and expected external actions, and performs no push or PR mutation. Local preview bookkeeping is distinct from external publication. The new mutation route requires explicit consent bound to repository name/ID, head/base and the current integration commit, plus authenticated push permission and current project/lifecycle authorization.

The comparison base is the recorded run base or, when an older retry left that field absent, the verified first integration journal's before-commit. Its source is exposed. Missing evidence is labelled incomplete and blocks the V2 publication route; it is not presented as a complete empty change list. A first publication validates the resolved preview base rather than requiring a base already stored by an earlier publication.

Journal-backed P08 runs revalidate completed task state, published candidate identity, actual automatic review decisions, unresolved human change requests and current required combined-check evidence before delivery admission and again before the push. Histories without modern journals retain their V1 compatibility contract. A new human publication fence cannot be accepted once integration or delivery already owns its publication phase; existing unresolved findings block that phase.

The existing delivery coordinator claims operations durably before pushing or creating/linking a PR. The new V2 endpoint validates the effective push URL (including rewrite/pushurl configuration); local or foreign destinations and multiple push URLs are refused. Legacy transport fixtures preserve their explicit local redirect contract. Core delivery pushes the approved immutable SHA to its exact factory/run ref with no force and no followed tags, even if a local branch later moves. This does not configure upstream tracking as an incidental publication side effect. Duplicate requests reuse recorded ownership. A timeout or unconfirmed effect requires read-only reconciliation; no blind duplicate PR creation or force push is introduced. A local integration branch is not a published remote branch. Publication is displayed only from confirmed remote/durable delivery evidence.

Only a run's recorded published PR may be updated through the new update route. It validates repository identity, expected local/remote head, remote revision and authorization. Title/body use bounded structured inputs and an exclusive body file. Draft transitions use supported gh operations. The result is read back and verified. A partial/uncertain update requires inspection; retries reconcile desired state instead of blindly repeating mutation. Updating text/draft does not rewrite the published branch.

Human/local automated review records are separate from remote approvals. There is no new remote review submission, automatic merge or deployment action.

## Qualification boundary

Deterministic provider tests exercise the production adapter and delivery boundaries, not a simulated UI as proof of publication. Real read-only qualification uses only an authorized repository/account. A live PR creation/update test requires separately authorized disposable remote scope. Without it, report **LIVE GITHUB WRITE ACCEPTANCE: NOT QUALIFIED — NO AUTHORIZED REMOTE MUTATION**. No ASF development-repository push or PR is authorized by P12.

The positive V2 API contract fixture uses real authentication/CSRF, project routing, immutable Git objects, the production delivery coordinator, a controlled `gh` executable and a repository-configured SSH relay terminating only at an owned local bare repository. It verifies a side-effect-free preview, identity denial, exact approved-SHA publication and duplicate prevention. That local transport result is not a live GitHub PR.
