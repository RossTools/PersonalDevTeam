# GitLab

How PDT talks to GitLab (PDT-246): the GitLab forge over GitLab's REST API (v4), for projects on gitlab.com and on
self-hosted GitLab servers, with GitLab CI as its CI provider. Nothing runs the GitLab CLI (`glab`). It is an optional
plugin (crates/pdt-int-gitlab, pdtd's `gitlab` feature): turn it on in Settings → Plugins → GitLab and sign in there.
What the SDLC does with a project's forge is in [tickets.md](tickets.md) ("Pull requests and CI") and
[review.md](review.md) ("Merge"); the GitHub forge it mirrors is [github.md](github.md).

## Which projects use it

- While the plugin is on, `GitlabPlugin` claims remotes on gitlab.com (and `www.gitlab.com`) and on each self-hosted
  host PDT has a GitLab login for: signing in with a server's API URL is how a host is added, and disconnecting its
  last account removes it. A project whose `origin` is on one of them has the GitLab forge, unless
  `.pdt/project.toml` names another (`forge = "gitlab"` names it outright, docs/run-config.md "Forge").
- The repository is the project's full path, any depth of groups (`group/sub/app`); the API names it URL-encoded
  (`projects/group%2Fsub%2Fapp`). A remote pointing inside a project (`…/-/tree/main`) is not one.
- Links: the repository at the account's web URL (a server's port or path kept), branches at `/-/tree/{branch}`, and a
  new merge request at `/-/merge_requests/new?merge_request[source_branch]=…&merge_request[target_branch]=…`.
- The plugin can't be turned off while a project's forge is GitLab (409 `in_use`, as for GitHub, PDT-251).
- Requests go through the forges' shared transport (`pdt_forge::transport`): pdtd's reqwest client, or none at all with
  `PDT_GITHUB_OFFLINE=1` (which takes every forge offline); the tests use recorded answers.

## The login

- Settings → Plugins → GitLab lists the accounts, one per host, in the forge logins (`gitlab:<host>`, docs/reference
  github.md "The login"). The form takes an API URL (gitlab.com's by default; a bare server URL gets `/api/v4`) and a
  token: a personal, group or project access token with the `api` scope (`read_api` can't merge, comment or retry).
  **Test** asks `GET /user` who it is, and `GET /personal_access_tokens/self` its scopes.
- The OAuth device flow (GitLab 17.1+: `POST /oauth/authorize_device`, then `POST /oauth/token` until the code is
  entered) needs an OAuth application with the device flow on. PDT has none registered on gitlab.com yet, so there it
  works only with `PDT_GITLAB_CLIENT_ID` set; on any host the form takes the client ID of the user's own application.
  GitLab's OAuth tokens expire after two hours: PDT renews them with their refresh token 15 minutes ahead (and once
  more on a 401), and stores the new pair at once (GitLab's refresh tokens work once).
- A 401 that `GET /user` gets too marks that account refused: nothing is sent to the host until the user signs in
  again, the plugin shows Error, and the Inbox asks to reconnect. The 5-minute login check asks `GET /user` of every
  account. A used-up rate limit (`RateLimit-Remaining: 0` until `RateLimit-Reset`, or a 429's `Retry-After`) stops
  every call to that host until it resets.

## Merge requests

- A branch's change request is the newest open merge request from it in the project itself (a fork's of the same
  branch name is left out), else its newest: `iid` is its number, `sha` its head, `target_branch` its base.
  `has_conflicts` (or `detailed_merge_status: conflict`) is a conflict, `checking`/`unchecked`/`preparing` is not yet
  known, `need_rebase` is behind its base, `merge_when_pipeline_succeeds` is auto-merge. Opening one sends
  `Draft: <title>` for a draft.
- Comments are the merge request's discussions (system notes left out): a `DiffNote` is a line comment (its
  `position.new_path` and `new_line`), and a reply goes into the comment's discussion. Links are the notes'
  (`…/-/merge_requests/<iid>#note_<id>`). An author is an insider when they are a member of the project at Developer
  or above (`GET members/all/:id`, asked once per user).
- Reviews are approvals (`approved`), and reviewers' states from GitLab 17 (`requested_changes`, `reviewed`). Approving
  calls `…/approve` (its text as a comment); GitLab's API can't request changes, so that review is a comment saying so.
- Files and stats come from `…/diffs` (lines counted from each diff; a too-large one has no patch).

## Merging and merge trains

- Merge settings come from the project (`GET /projects/:id`, kept 5 minutes): squash unless `squash_option` is
  `never`, a merge commit with `merge_method: merge`, else a rebase (`ff`, `rebase_merge`).
- Without a merge train, Merge is `PUT …/merge_requests/:iid/merge` with `auto_merge` (and, for servers before 17.11,
  `merge_when_pipeline_succeeds`): GitLab merges once the pipeline succeeds, or at once with no pipeline. With the
  approved head given it sends `sha`, and GitLab refuses (409) once the head has moved.
- **Merge trains** (Premium and Ultimate) are found per project: on when `merge_trains_enabled` and
  `merge_pipelines_enabled` (merged-results pipelines) both are. Then the train is the base branch's merge queue
  (`MergeSettings::queue`, "merge train" in PDT's words): Merge adds the merge request to it
  (`POST merge_trains/merge_requests/:iid`), its car's status says whether it is on the train
  (`idle`, `stale`, `fresh`, `merging`) and its place in `merge_trains/<base>` its position, and the newest
  merge-train system note that took it off ("removed this merge request from the merge train because the pipeline
  did not succeed") is the queue's removal: failed checks or a conflict, by the train when it gives a reason, else by
  hand. The train's pipelines run on `refs/merge-requests/<iid>/train` (`Forge::queue_ref`).

## GitLab CI

- `GitlabCi` (provider id `gitlab_ci`, "GitLab CI") reads the same signed-in client. Every pushed branch gets a
  pipeline, so it is read before a branch has a merge request (`CiCapabilities::branches`). A repository has it when
  a commit has `.gitlab-ci.yml`.
- A head's runs are its pipelines (`GET pipelines?sha=<head>`: the branch's and its merge request's). With
  merged-results pipelines on, the open merge request's `head_pipeline` runs on a merge commit: it counts as the
  head's. A pipeline's `name` (else `pipeline`) is its run's name; `success` and `skipped` pass, `failed` fails,
  `canceled` is cancelled, `running` runs, and everything else (`created`, `pending`, `manual`: waiting for someone to
  start a job it needs) is queued.
- Jobs come from `pipelines/:id/jobs`; a failure the pipeline allows (`allow_failure`) is not one to fix. Logs are
  `jobs/:id/trace`, cut to their last 200 lines (30,000 characters) without GitLab's section markers, overwritten
  progress or terminal colours; a job with none says so.
- Retrying failed jobs is `pipelines/:id/retry`; retrying all of a branch's pipeline is a new pipeline on the branch
  (a merge request's pipeline can only be retried). Starting one (`trigger`) is `POST /pipeline` on the ref with the
  inputs as CI/CD variables (GitLab has one pipeline configuration per project, so the workflow only names it).
  Merge-train runs are the merge requests' recent pipelines (`source=merge_request_event`).

## Not covered yet

- GitLab issues are read and written through the forge tools (`issue_view`, `issue_comment`; `issue_create` is the
  GitHub plugin's, and files on the project's forge while that plugin is on). An issue's id
  is its `iid`, a duplicate is closed with `/duplicate #<iid>`, and labels the project lacks are made), but only
  GitHub's issues link to tickets and feed the issue triage, and the `github.*` workflow steps are GitHub's.
- Releases are read (`releases/permalink/latest`, `releases/:tag`; one not out yet, `upcoming_release`, is none), for
  a flow that names the GitLab forge; the built-in release workflow is GitHub's.
