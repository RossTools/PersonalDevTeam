# GitHub

How PDT talks to GitHub (PDT-177): the GitHub forge over GitHub's REST and GraphQL APIs, signed in with an in-app
login. Nothing runs the GitHub CLI (`gh`). Which projects use it, and what the SDLC does with it, is in
[tickets.md](tickets.md) ("PR and CI tracking") and [review.md](review.md) ("Merge").

## The forge

- `crates/pdt-github`: `GithubApi` implements `pdt_forge::Forge` (every capability). It sends one request at a time
  through a `Transport`: pdtd's `ReqwestTransport` (reqwest on pdtd's runtime; blocking, so callers use
  `spawn_blocking`), `transport::Offline` (no network: `AppState::new`'s forge, `PDT_GITHUB_OFFLINE=1`) or the tests'
  `transport::testing::Recorded` (recorded answers matched by method and URL start; every request kept).
- Pull requests, merge settings and merges go through GraphQL (`/graphql`, Enterprise `/api/graphql`); Actions runs,
  jobs, logs, comments, reviews and issues through REST (`X-GitHub-Api-Version: 2022-11-28`), as do releases
  (`releases/latest`, `releases/tags/{tag}`; drafts and pre-releases don't count, a 404 is none) and starting a
  workflow (`POST actions/workflows/{file}/dispatches`; the forge's `releases` capability, PDT-194).
- Merging turns on auto-merge (`enablePullRequestAutoMerge`, with the strategy unless a merge queue picks it). A pull
  request GitHub calls clean merges at once instead: with no strategy it joins the merge queue
  (`enqueuePullRequest`), else `PUT …/pulls/<n>/merge`.
- Failed logs (`Forge::failed_logs`, the `ci_failed_logs` tool): the run's latest attempt's failed, timed-out or
  never-started jobs, each job's log through `…/actions/jobs/<id>/logs` (a redirect to storage; reqwest drops the
  token when it leaves GitHub's host), cut to its last 200 lines (30,000 characters), timestamps and terminal colours
  dropped. A log GitHub no longer has says so instead of failing the call.
- **Redirects** (PDT-195): the transport follows at most 3, so a project repository that was renamed or transferred
  keeps working (GitHub answers 301 for a GET and 307 for the rest, so a POST keeps its body). The token goes along
  only while a redirect stays on the same host, scheme and port; reqwest drops it at the first hop elsewhere (a job
  log's storage link). A permanent move on the API host is logged with both locations, without query strings.
- **Conditional requests**: every API GET but logs sends the ETag of its last answer (`If-None-Match`); a 304 gives
  the cached body and costs no rate limit. The cache holds 500 answers and is cleared when the login changes.
- **Rate limits**: an answer with `x-ratelimit-remaining: 0` records `x-ratelimit-reset`; a 403 or 429 with
  `retry-after` (the secondary limit) records now plus that. Until then every call fails at once with "GitHub's rate
  limit is used up until HH:MM UTC", without sending anything.
- Errors: no login is `ForgeError::Unavailable` ("GitHub is not connected: …"), as is a 401 ("GitHub no longer accepts
  PDT's login: connect GitHub again"); anything else is `Failed` with GitHub's status and message.

## The login

- Settings › This host › GitHub (`web/src/host/GithubLogin.tsx`; pdt-api `github_login.rs`). The login is a JSON
  `Account` (`token`, `apiUrl`, `login`) in the OS keychain only, under `github-login` for the stable instance's
  default data dir and `github-login:<hash of the data dir>` for any other (`secrets::github_login_account`, as the
  update token's), so a dev instance never reads or replaces the stable one's. pdtd reads it at startup in the
  background (`github_login::load`). The token never leaves pdtd: the API returns who it signs in as, never the token.
- **Connect GitHub** (the OAuth device flow; `POST /api/github/login/device`, body `{apiUrl?, clientId?}`): PDT asks
  GitHub for a code (`<site>/login/device/code`), the page shows it and the link to enter it at, and a background task
  polls `<site>/login/oauth/access_token` at GitHub's interval (slowing down when told) until the user authorizes,
  denies, or the code expires (15 minutes). The token is checked with `GET /user` and saved. `DELETE
  /api/github/login/device` stops waiting; starting again replaces the old flow.
- **Paste a token** (`POST /api/github/login/token`, body `{token, apiUrl?}`): checked with `GET /user` (a 401 is
  refused) and saved. `gh auth token` prints the GitHub CLI's, for a user who has one; PDT itself never runs `gh`.
- `GET /api/github/login`: `connected`, `login`, `apiUrl`, `webUrl`, `deviceFlow` (a client ID is set), `scopes`,
  `pending` (the code to enter, while waiting), `error` (why the last login failed). `DELETE /api/github/login`
  disconnects: the keychain entry goes and the forge signs out (GitHub keeps the app's authorization until the user
  revokes it under Settings › Applications on GitHub).
- **Scopes**: `repo`. It covers what PDT does in public and private repositories: reading pull requests, reviews,
  comments, issues, Actions runs, jobs and logs, releases; commenting, labelling and closing issues; turning on
  auto-merge and merging; starting a workflow by hand (the built-in "Cut a release", PDT-194). A fine-grained token
  instead needs, on the repositories PDT works with: Pull requests, Issues and Contents read and write, Actions read
  (and write to start a workflow) and Metadata read.
- **GitHub Enterprise Server**: give the host's API URL (`https://github.example.com/api/v3`). The site is the URL
  without `/api/v3`, GraphQL is `<site>/api/graphql`, and a project whose `origin` is on that host uses the forge
  (`git_url::github_repo_on`). The device flow there needs an OAuth App registered on that host (its client ID in the
  form); PDT's own is github.com's.

### Registering PDT's OAuth App

The device flow needs an OAuth App's client ID: `pdt_core::brand::GITHUB_OAUTH_CLIENT_ID` (with it empty, Settings
offers only a pasted token). `PDT_GITHUB_CLIENT_ID` overrides it for one pdtd (a dev instance, or another app).

PDT's app is registered (PDT-190): owned by the **RossTools** organization, Device Flow enabled, Client ID
`Ov23litfc05CvYvo8kR5` (not a secret: it ships in the binary; there is no client secret). To register another one (a
fork, or an Enterprise host):

1. On GitHub: Settings › Developer settings › OAuth Apps › New OAuth App (or the organization's, to own it there).
2. Application name: Personal Dev Team. Homepage URL: the project's repository. Authorization callback URL: any URL
   (the device flow never calls it), e.g. the homepage.
3. Register it, then tick **Enable Device Flow** and save. No client secret is needed: the device flow doesn't use one.
4. Put the Client ID in `GITHUB_OAUTH_CLIENT_ID` (crates/pdt-core/src/brand.rs) and release (done for PDT's app).
5. At the cutover each host's user opens Settings › This host › GitHub, clicks Connect GitHub, enters the code on
   GitHub and authorizes the app.

Private repositories of an organization: the organization has to grant the app access (Organization settings › Third-
party access › OAuth app policy), or an owner approves it when a member asks at first sign-in. Until then the token
works but that organization's private repositories answer 404.

## Not connected

- Every forge call without a login fails and marks the forge as wanted (`Forge::login_needed`). While it is, the Inbox
  lists **Connect GitHub** (kind `connect`, group Unheard notices; not stored, never dismissed): Enter opens Settings
  › This host › GitHub. It goes once a login is saved.
- A login GitHub refuses (it answers **401**: the token was revoked or expired; PDT-212) is kept but marked rejected,
  with the time (`GithubApi::rejected_at`, `Forge::login_rejected_at`; in memory, so a restart finds out again on its
  first call). While it is, no call reaches GitHub: every one fails as "GitHub no longer accepts PDT's login" at once,
  so the 60 s pollers back off without spending the rate limit. `GET /api/github/login` reports it as `rejectedAt`
  (still `connected`), Settings › This host › GitHub says "GitHub no longer accepts this login" with Reconnect, and
  the same single Inbox item reads **Reconnect GitHub** (`body.reason: "rejected"`, created at the refusal). Saving
  a login (device flow or pasted token) clears it, takes the item out of the Inbox and runs the PR and issue checks
  at once. A 403 (missing scopes, or no access to one repository) is not a rejected login: it stays that PR's or
  project's error.
- The Connect item is a needs-you notification (Unheard notice, warning) like other blocking items, so it shows
  as the user's notification settings say; it clears when the item goes.

## The forge tools

Agents read and answer on GitHub through PDT's tools instead of `gh` ([agents.md](agents.md), "Agent sessions"):
`change_request_view`, `change_request_comment` (`replyTo` answers a review comment), `change_request_open` (`POST
/repos/{repo}/pulls`; GitHub's 422 for an unpushed head, or one with nothing new, names that), `change_request_merge`
(auto-merge or the queue again after a queue drop, PDT-185), `ci_runs`, `ci_failed_logs`, `issue_view` and
`issue_comment`, on the session's project forge and branch. They are in the default tool set, so the
Coder, Reviewer and QA have them; the PM has `issue_view`. Agents still push with plain `git`.
