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
  (`releases/latest`, `releases/tags/{tag}`; drafts and pre-releases don't count, a 404 is none; `newest_release`,
  the first non-draft of `releases`, pre-releases included, for an experimental release, PDT-210) and starting a
  workflow (`POST actions/workflows/{file}/dispatches`; GitHub Actions' `trigger`, PDT-194).
- Merging turns on auto-merge (`enablePullRequestAutoMerge`, with the strategy unless a merge queue picks it). A pull
  request GitHub calls clean merges at once instead: with no strategy it joins the merge queue
  (`enqueuePullRequest`), else `PUT …/pulls/<n>/merge`.
- GitHub Actions is GitHub's CI provider (PDT-242, `pdt_github::GithubActions`, registered by the GitHub plugin as
  `ForgePlugin::ci`), over the same login: runs, jobs, logs, re-runs (`…/rerun`, `…/rerun-failed-jobs`), starting a
  workflow and merge-queue runs.
- Failed logs (`CiProvider::jobs` and `job_log_tail`, the `ci_failed_logs` tool): the run's attempt's failed, timed-out or
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

- GitHub's plugin page, Settings › Plugins › GitHub (PDT-228; its connections are its logins:
  `web/src/plugins/ForgeLogin.tsx`; pdt-api `forge_login.rs`). The old path `/settings/host/github` goes there.
- **An account per host** (PDT-244): PDT keeps one login per forge kind and host, keyed `<kind>:<host>`
  (`github:github.com`, `github:ghe.example.com`), so github.com and any number of Enterprise Servers are signed in at
  once. Each is a JSON `ForgeAccount` (`token`, `apiUrl`, `login`, `signedInAt`; `expiresAt` and `refresh` for a token
  that expires, see "Expiring tokens"), and all of them are one
  `pdt_forge::login::AccountStore` in the OS keychain only, under `forge-logins` for the stable instance's default
  data dir and `forge-logins:<hash of the data dir>` for any other (`secrets::forge_logins_account`, as the update
  token's), so a dev instance never reads or replaces the stable one's. pdtd reads it at startup in the background
  (`forge_login::load`) and signs each forge plugin in for each host (`ForgeLogin::set_account`). The GitHub plugin
  keeps one `GithubApi` per host (github.com's always; an Enterprise Server's from its first account), each with its
  own ETag cache, rate limit and refusal; a project uses the account of its `origin`'s host
  (`ForgePlugin::forge_on`, `ci_on`). The token never leaves pdtd: the API returns who it signs in as, never the token.
- **The login from before PDT-244** (one GitHub login, under `github-login[:<hash>]`): the first read finds no
  `forge-logins` entry, copies it in as the account of its API URL's host and writes that, with no new sign-in. The
  old entry stays, for an older PDT rolled back to, until the user disconnects that host.
- **Every forge's login** is the plugin's `ForgeLogin` (`ForgePlugin::login`, pdt-forge `login.rs`): what its form asks
  (`LoginSpec`: the public host's API URL, prefilled; an example of a self-hosted one; the scopes; help on tokens;
  whether it has a device flow), checking a token (`whoami`), the device flow where it has one, and its accounts. The
  API is the same for every forge, under `/api/forges/{kind}/login` (an unknown kind, or one without a login, is
  `404 unknown_forge`):
  - `GET`: `kind`, `label`, `defaultApiUrl`, `apiUrlExample`, `scopes`, `tokenHelp`, `hasDeviceFlow`, `deviceFlow`
    (PDT has an OAuth App client ID for the public host), `accounts` (`host`, `apiUrl`, `webUrl`, `login`,
    `signedInAt`, `rejectedAt`, `expiresAt`, `renews`; the public host's first), `pending` (the code to enter and its `apiUrl`, while
    waiting), `error` (why the last login failed).
  - **Test** (`POST …/test`, body `{token, apiUrl?}`): who the token signs in as on that host (`host`, `apiUrl`,
    `webUrl`, `login`, `scopes`), without saving it; a token the forge refuses is `400 invalid_token`.
  - **Paste a token** (`POST …/token`, same body): checked the same way (GitHub: `GET /user`; a 401 is refused), then
    saved as the account of the API URL's host, replacing that host's. `gh auth token` prints the GitHub CLI's, for a
    user who has one; PDT itself never runs `gh`.
  - **Connect GitHub** (the OAuth device flow; `POST …/device`, body `{apiUrl?, clientId?}`; `400 no_device_flow` on a
    forge without one): PDT asks GitHub for a code (`<site>/login/device/code`), the page shows it and the link to
    enter it at, and a background task polls `<site>/login/oauth/access_token` at GitHub's interval (slowing down
    when told) until the user authorizes, denies, or the code expires (15 minutes). The token is checked with `GET
    /user` and saved as that host's account. `DELETE …/device` stops waiting; starting again replaces the old flow.
  - `DELETE …/accounts/{host}` disconnects one host (`404 no_account` without one): its account leaves the keychain
    and that host's forge signs out (GitHub keeps the app's authorization until the user revokes it under Settings ›
    Applications on GitHub). The others stay.
- The page lists each account (who, on which host, when its token expires if it does, Disconnect…), Connect GitHub (the device flow on github.com), and
  a form for any host: its API URL (github.com's prefilled), a token, Test (who it signs in as, and its scopes) and
  Save; for another host with the device flow, that host's OAuth App client ID. All connections lists each account.
- **Scopes**: `repo`. It covers what PDT does in public and private repositories: reading pull requests, reviews,
  comments, issues, Actions runs, jobs and logs, releases; commenting, labelling and closing issues; turning on
  auto-merge and merging; starting a workflow by hand (the built-in "Cut a release", PDT-194). A fine-grained token
  instead needs, on the repositories PDT works with: Pull requests, Issues and Contents read and write, Actions read
  (and write to start a workflow) and Metadata read.
- Every sign-in is logged at info (PDT-252): the instance (its keychain account and data dir), how (device flow or
  pasted token), the OAuth App's client ID, who, the API and the token's scopes; pdtd's startup logs the stored login
  and how long ago it was made. Never the token.
- **GitHub Enterprise Server**: give the host's API URL (`https://github.example.com/api/v3`). The site is the URL
  without `/api/v3`, GraphQL is `<site>/api/graphql`, and a project whose `origin` is on that host uses the forge
  as that host's account (the GitHub forge plugin claims each host it has an account for,
  `GithubPlugin::claims_host`; without one the project has no forge). The device flow there needs an OAuth App
  registered on that host (its client ID in the form); PDT's own is github.com's.

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
5. At the cutover each host's user opens Settings › Plugins › GitHub, clicks Connect GitHub, enters the code on
   GitHub and authorizes the app.

PDT's app has **token expiration off** (the app's token expiration setting on GitHub, under Optional features;
turned off on 2026-10-29, PDT-260), so its tokens don't expire: a login on it stores no expiry and is never
renewed. Before that, every device-flow login died 8 hours after sign-in (PDT-252). An app registered with expiration
on (GitHub's default for new apps on some accounts) still works: see "Expiring tokens".

Private repositories of an organization: the organization has to grant the app access (Organization settings › Third-
party access › OAuth app policy), or an owner approves it when a member asks at first sign-in. Until then the token
works but that organization's private repositories answer 404.

### Expiring tokens

An OAuth App with token expiration on (an app of your own, or one on an Enterprise Server) issues tokens that expire
after 8 hours, with a refresh token good for about 6 months. PDT-260 keeps them working:

- The device flow's answer (`expires_in`, `refresh_token`, `refresh_token_expires_in`) is kept with the account in the
  keychain: `expiresAt` (Unix ms) and `refresh` (`token`, `clientId`, the app it came from; `expiresAt`). Settings ›
  Plugins › GitHub shows when the token expires and that PDT renews it. A pasted token never renews.
- Within 15 minutes of expiry the next call first asks `<site>/login/oauth/access_token` with
  `grant_type=refresh_token` (client ID only, as the device flow) for a new token (`GithubApi::renew`). The 5-minute
  login check makes that call even when nothing else does, so an idle PDT renews in time, and one that slept past the
  expiry renews on waking. GitHub issues a new refresh token each time and the old one stops working, so the new
  account is stored in the keychain at once (`ForgeLogin::on_renewed`, set by pdt-api `forge_login`; only over the
  account it renewed, never over a newer sign-in). One renewal runs at a time per host; a call that waited for it
  uses its token.
- A 401 on a login that renews gets one renewal and one more try before it counts as a refusal.
- A refresh token GitHub refuses (`bad_refresh_token`: revoked, already used, or the app's authorization withdrawn),
  or one past its own expiry, marks the login rejected: **Reconnect GitHub**, as for any refused login. No answer (a
  network failure, a 5xx) marks nothing: the old token serves until it expires, and the next try waits a minute.
- Each renewal is logged at info (who, the host, the client ID, when the new token and refresh token expire), each
  failure at warn. Never the tokens.
- To check an app of your own: on GitHub, Settings › Developer settings › OAuth Apps › the app › Optional features, its
  **token expiration** setting. On, its tokens expire and PDT renews them as above; off, they don't. Changing it
  affects new logins only: connect again afterwards.

## Not connected

- Every forge call without a login fails and marks the forge as wanted (`Forge::login_needed`). While it is, the Inbox
  lists **Connect GitHub** (kind `connect`, id `connect:github`, group Unheard notices; not stored, never dismissed):
  Enter opens Settings › Plugins › GitHub. It goes once a login is saved. Each forge plugin in use gets its own item
  (`connect:<kind>`, PDT-241), named with its label, and each account on another host than the public one its own
  too (PDT-244: `connect:<kind>:<host>`, **Reconnect GitHub on ghe.example.com**, `body.host`), so a refused
  Enterprise Server login asks for itself while github.com's carries on. The plugin's status is Error while any of
  its accounts is refused.
- A login GitHub refuses (it answers **401**: the token was revoked or expired; PDT-212) is kept but marked rejected,
  with the time (`GithubApi::rejected_at`, `Forge::login_rejected_at`; in memory, so a restart finds out again on its
  first call). While it is, no call reaches GitHub: every one fails as "GitHub no longer accepts PDT's login" at once,
  so the 60 s pollers back off without spending the rate limit. `GET /api/forges/github/login` reports it as that
  account's `rejectedAt` (still listed), Settings › Plugins › GitHub says "GitHub on <host> no longer accepts this
  login" (with Reconnect for github.com), and
  the same single Inbox item reads **Reconnect GitHub** (`body.reason: "rejected"`, created at the refusal). Saving
  a login (device flow or pasted token) clears it, takes the item out of the Inbox and runs the PR and issue checks
  at once. A 403 (missing scopes, or no access to one repository) is not a rejected login: it stays that PR's or
  project's error.
- A 401 marks the login only when GitHub means it (PDT-252): the 401 must come from the API's own host (one from a
  redirect's other host, such as a job log's storage, is that call's error), and `GET /user`, asked at once, must
  answer 401 too. Otherwise the call fails with GitHub's status and message and the login stays.
- **The login check** (`forge_login::check`, the `forge logins` loop, every 5 minutes, of every account;
  `ForgeLogin::check_accounts`, GitHub's `GithubApi::check_login` per host):
  `GET /rate_limit` (it costs no rate limit), so the time GitHub stops accepting the login is in the log to within 5
  minutes even when nothing else calls GitHub. While the login is marked rejected it asks `GET /user` instead, and
  when GitHub accepts the login again the mark goes, the Inbox item with it, and the PR and issue checks run at once.
- **Diagnosing a refusal**: every 401 (warn) and 403 (info) is logged with the method, the endpoint, the URL that
  answered after redirects (no query), GitHub's message, `x-github-request-id`, `x-github-sso`, `www-authenticate`,
  `x-oauth-scopes`, the login's age and when GitHub last accepted it. The forge keeps its last 50 calls in memory
  (method, path, status, time; `GithubApi::recent_calls`) and logs them at a login's first 401. Never the token.
- The Connect item is a needs-you notification (Unheard notice, warning) like other blocking items, so it shows
  as the user's notification settings say; it clears when the item goes.

## The forge tools

Agents read and answer on GitHub through PDT's tools instead of `gh` ([agents.md](agents.md), "Agent sessions"):
`change_request_view`, `change_request_comment` (`replyTo` answers a review comment), `change_request_open` (`POST
/repos/{repo}/pulls`; GitHub's 422 for an unpushed head, or one with nothing new, names that), `change_request_merge`
(auto-merge or the queue again after a queue drop, PDT-185), `ci_runs`, `ci_failed_logs`, `issue_view` and
`issue_comment`, on the session's project forge and branch. They are in the default tool set, so the
Coder, Reviewer and QA have them; the PM has `issue_view`. Agents still push with plain `git`. `issue_create` files an
issue (below); it asks the user first.

## Filing issues (PDT-221)

`Forge::create_issue` (title, body, labels; `Capabilities::issues`) opens an issue as the forge's login: GitHub's
`POST /repos/{repo}/issues` (labels the login can't set, or the repository lacks, are dropped by GitHub; a body over
65536 characters is cut), and `FakeForge` in tests (numbered from 500). pdt-api `issues.rs` files one in a project's
repository through its project forge (only GitHub's: the link is `owner/name#12`), and with a ticket links the new
issue to it (pdt-store `link_external`: a `linked` event `{system, key, url}`). The ticket keeps its source: the
issue came from it, so it does not become `external` ([tickets.md](tickets.md) "Sources and the Todo gate"). A
ticket links one GitHub issue (`409 already_linked`).

- `GET /api/projects/{id}/forge`: `{kind, repo, issues}` (null without a forge); `issues` says whether issues can be
  filed there.
- `POST /api/projects/{id}/issues` (`{title, body, labels, ticketId}` → `{repo, number, issue, url, ticketKey}`):
  only from PDT's web app (its UI token, PDT-204; `403 not_the_ui` otherwise), since it publishes as the user. A forge
  error is `502 forge_error` (`503 forge_unavailable` when it can't be reached).
- The UI: **Create GitHub issue…** in a ticket's actions menu (when the project's forge has issues and the ticket
  links none) opens a form with its title and description, editable; the command palette's **New issue** files one in
  the current project's repository.
- The `github.create_issue` node and the `issue_create` tool are one action (pdt-api `integrations/github.rs`):
  `ticket` (its project's repository; its title and description unless `title`/`body` are given) or `project`
  (an agent's own project when left out), `title`, `body`, `labels`. The node leaves by `filed` with `{repo, number,
  issue, url, ticketKey}`. The tool asks before each call (the PM: unless the ticket's or project's policy lets it act);
  it is in the default tool set and the PM's.
