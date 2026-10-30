# Changelog

What changed for people who use PDT, newest first. Each section is compiled from the changelog fragments the pull
requests added (`.changes/`; docs/SDLC.md "Changelog") by `scripts/changelog.sh <version>`.

## 0.4.0 (2026-10-30)

### Added

- Grok Build (xAI's `grok` CLI) is now a built-in agent: once `grok` is installed and signed in, pick "Grok" anywhere you pick an agent, including personas for workflow steps. It runs as a conversation, and PDT keeps Ask, Accept edits, Plan and Bypass for it by answering Grok's permission requests.
- Cursor is now a built-in agent: once the Cursor CLI is installed and signed in (`agent login`), pick "Cursor" anywhere you pick an agent (a new tab, ticket Start, personas) to run it as a conversation, with PDT's tools, its command and edit approvals, its questions and its plans in PDT. Cursor edits files without asking, so it offers Accept edits, Plan and Bypass, and a session or persona set to Ask runs as Accept edits (commands still ask). On Cursor's Free plan, pick the Auto model.
- OhMyPi (`omp`) is now a built-in agent: once `omp` is installed and has a model, pick "OhMyPi" anywhere you pick an agent (a new tab, ticket Start, personas) to run it as a conversation, with its questions in PDT. It asks before commands, deletes and moves; its other file edits run without asking, even in Ask mode. Errors from ACP agents now show their details (such as omp's "No model selected") instead of just "Internal error".
- Google Antigravity is a built-in agent. In Settings → Agents, Install gets Google's Antigravity ACP server (checked against the ACP registry), Sign in uses your Google account, and then you can pick Antigravity for conversations and personas, with its models in the model picker.
- GitLab plugin: projects on gitlab.com or your own GitLab server can use GitLab as their forge. Turn it on in Settings → Plugins → GitLab and sign in with an access token; PDT then follows each ticket's merge request and its GitLab CI pipelines, tells agents about failed jobs with their logs, and merges when the pipeline succeeds (on the merge train, where the project has merge trains on).
- The GitHub plugin can be turned off in Settings → Plugins when none of your projects uses GitHub as its forge: PDT stops using GitHub for pull requests, CI and issues, and its workflow steps, triggers, built-in workflows and the Connect GitHub notice go away until you turn it back on (updates and bug reports still go through GitHub). While a project still uses GitHub, its page says which one.
- Settings → Agents → Default models sets the model each agent starts with (for example Claude on Opus): new conversations, workflow steps, the PM and fresh Claude and Codex terminals use it unless the conversation or its persona picks a model, and the composer's picker shows it as "Default (Opus)".
- A security policy (SECURITY.md): report vulnerabilities privately through GitHub's "Report a vulnerability" or to security@pdt.dev, never in a public issue.
- Workflows can react to pull requests PDT didn't open on your projects' GitHub repositories: new triggers fire when one is opened (from a fork, by Dependabot or another bot), pushed to, or closed. They carry the author's kind, fork, size and, for Dependabot, whether the update is major, minor or patch; their title and body are treated as outside input, like new issues.
- A new workflow template, Review outside pull requests: each pull request someone outside your repository (or a bot) opens gets a ticket, a locked-down PR Reviewer reviews it from GitHub's diff without checking it out, and your Inbox asks you to Approve & merge, Request changes or Close; nothing is posted on the pull request until you choose. Dependabot's patch and minor updates merge on their own once CI passes. It is on by default for PDT's own repository.
- Settings → Agents has an on/off switch for each agent, including installed ones and ACP agents. A switched-off agent is no longer offered anywhere (ticket Start, new tab, the palette, personas, Get started) and no new session or workflow step starts on it; sessions already running carry on.
- The website has a Plugins page: every plugin PDT comes with, by category like Settings › Plugins, with a page for each saying what it needs, its workflow triggers and steps and its tools. The media plugins have a page of their own, and "Request a plugin" opens an issue.
- Install PDT with one command: `curl -fsSL https://pdt.dev/install.sh | sh` on macOS and Linux, `irm https://pdt.dev/install.ps1 | iex` on Windows. It picks the right release for your system (or a nightly build, or an exact version), checks it against the release's checksums and sets PDT up to start at login, without admin rights. The website shows the command for your system, with tabs for the others.

### Changed

- The favicon's blue dot now lights up only for conversations and terminals you started (and unread PM replies): agentic ticket steps (Coder, Reviewer, QA…) finishing no longer turn it on. They still show the red dot when they need you.
- Finished Triage and other workflow-step sessions with no ticket no longer pile up in the Agents list. They are kept: open one from its workflow run's step with the new Open session button.
- The website's Contributing section now has a short developer guide (how PDT is built, the rules a change must keep, and how its tests run) alongside how we work on PDT; the long per-part reference pages are gone.
- Experimental builds are now nightly builds: a new pre-release of the latest code is published every night it changed (`vX.Y.Z-nightly.YYYYMMDD`), and the newest 14 are kept. The update channel is now called **Nightly builds too**; a host set to the experimental channel is on it already and gets the next nightly without any change.
- The Agents list no longer shows the sessions a workflow starts for a ticket (Coder, Reviewer, QA and the like), running or finished. Open them from the ticket's Sessions or from the workflow run's step; one that needs you still shows in the Inbox. Sessions you start yourself stay listed.

### Fixed

- A message you send to a workflow step's session just as PDT closes it after the step's outcome is no longer lost: the session stays open until the agent has answered it, or reopens to take it.
- A workflow's Dry run no longer fails at a step that reads what an earlier action would have made (such as the ticket PDT bug triage imports): the action's output gets stand-ins like `<ticketId>`, and the dry run goes on to the end.
- On Windows, updates no longer fail over and over because a second pdt-host was running from the install folder and kept `pdtd.exe` open. Only one pdt-host now supervises a data folder, the updater stops anything still running from the install folder before installing, and an installer that leaves the old `pdtd.exe` in place is reported as such rather than as a failed health check. The keychain timeout message only mentions macOS on a Mac.
- Restarting a web run, or remote access restarting one that uses `$PDT_RUN_HOST`, keeps it on its port on Windows instead of sometimes moving it to another one because the stopped server had not let go of the port yet.
- A terminal Codex session that asks for approval again right after you answer it now shows "needs you" instead of sometimes staying "working".
- A GitHub login made with an OAuth App whose tokens expire (your own app, or one on GitHub Enterprise) no longer dies after 8 hours: PDT renews the token before it expires, and Settings › Plugins › GitHub shows when it does. If GitHub refuses to renew it, the Inbox asks you to reconnect.
- After an update that changes how PDT's host starts the server (like 0.2 → 0.3, which left Settings → Network → Port locked as "Set by PDT_PORT"), PDT now restarts the server once with the right settings, so you no longer need to restart it by hand.
- On a phone, writing a note or a todo's notes with the keyboard open no longer leaves an empty band between the last line and the keyboard, and tapping the space below the text puts the cursor at its end.
- Opening a ticket by its key (a `/tickets/PDT-18` link, a key in a description, a notification) no longer says "Ticket not found" for tickets that other tickets mention, and searching for a key lists that ticket first.
- In light mode, the ticket list's alternate rows are now easy to tell apart, and a hovered or selected row still stands out from them.
- The Changes view names who wrote each line comment: your own show as "You", and an agent's (such as the Reviewer's) show its name instead of "You".
- PDT's nightly releases are scheduled for 08:17 UTC instead of the top of the hour, which GitHub often delays or skips. When a night's nightly still isn't out by 10:00 UTC though `main` changed, a host that builds PDT now puts "Tonight's nightly didn't run" in the Inbox, with Start it, and the item clears itself once a nightly is published.

## 0.3.0 (2026-10-29)

### Added

- File a GitHub issue from a ticket (Create GitHub issue… in its actions menu, pre-filled with its title and description) or from the command palette (New issue). It goes to the project's repository under your GitHub login, and the ticket links the new issue. Agents and workflows can file issues too (`issue_create`, `github.create_issue`), and an agent asks you first.
- The Inbox's Updates can be cleared: **Clear all** empties the section, and each Update has its own Clear button (on its row, and when it is open), on desktop and phone. A cleared Update stays gone, and the same cause doesn't bring it back.
- Tickets that issue triage drafts now wait in the Inbox under Triaged tickets, with triage's notes and the issue they came from. Approve moves one to To do (and lets in a ticket that came from outside PDT), and Reject moves it to Won't do. Moving the ticket on the board takes it out of the Inbox. A project can turn this off under Pipeline and policies › Triaged tickets. Triage workflows you haven't edited pick up the new step on their own.
- Todos can have notes: click a todo's title to open it, write formatted notes in Markdown (they save as you type), and go back to the list. A todo with notes shows their first line and a notes icon, a repeating todo's next one keeps them, a due reminder opens its todo, and the PM sees the notes too.
- A new install opens on Get started: a checklist that finds git and your agents, connects GitHub (or skips it), adds your first project and picks the default agent, ticking each step off as it is done. Dismiss it, or reopen it from Settings › This host › Setup. An agent installed where PDT doesn't look can be given its path right in the checklist.
- Add any agent that speaks the Agent Client Protocol (ACP) in Settings → Agents → ACP agents, by its command, arguments and environment. It runs as a conversation wherever you pick an agent: a new tab, the palette, ticket Start, personas and workflow steps. PDT uses the modes the agent offers, or keeps the session's permission mode itself when it has none.
- PDT can be signed in to github.com and your GitHub Enterprise Servers at the same time, one login per host, and each project uses the login for the host its repository is on. On GitHub's plugin page you can add a login for any host with a personal access token, and **Test** shows who it signs in as before you save it; Connect GitHub with a code still works. Your current GitHub login carries over without signing in again, and a host that stops accepting its login gets its own Reconnect item in the Inbox.
- The local forge can push to a plain git remote on any host: with `[push] remote = "origin"` in `.pdt/project.toml`, PDT pushes each merge to the remote's base once CI passes, catches up and runs CI again when the remote moved on, and tells you in the Inbox when it can't. `branches = true` pushes ticket branches too.
- In the workflow editor, agent prompts, gate questions and messages are now Markdown editors, scripts and JSON fields are code editors, and each can open in a large dialog. A command step can hold a multi-line `script` (run with `sh`, `bash`, `zsh` or `pwsh`) instead of a program and its arguments.
- Jira connections work with Jira Server / Data Center (a personal access token) as well as Jira Cloud. The PM and workflow steps can now search with JQL, read, create, edit, assign and comment on issues, move them through transitions that need fields (Resolution, Story Points, …), and manage sprints, backlog sections and epics. Custom fields can be given friendly names on the connection, and Jira's own field names work too.

### Changed

- Report a bug asks what happened, what you expected and how to reproduce it in fields of their own, and the report lists PDT's version and OS under headings, the same as the bug form on PDT's GitHub repository. New issues on GitHub offer Bug and Feature forms.
- "Check for updates" in Settings → This host → Updates now says what it found: that you're up to date, which version is available, or why it couldn't check (for example no GitHub token saved, with a link to the token field).
- The workflow templates are about development now: Health watchdog, Brief follow-ups and Email processing are gone (workflows you already made from them keep working), and PDT bug triage is only offered and installed where a project is PDT's own repository.
- Settings has a **Plugins** page in place of Integrations and Connections: a catalog you can search and filter by status and category, shown as cards or a list, where each plugin says whether it is on, needs setup, is failing or is not in this build. Each plugin has its own page with its connections, what it adds (workflow steps, agent and PM tools, templates, project settings) and how to set it up; turning one on that needs a connection asks for one straight away. **All connections** lists every connection in one table, and the GitHub login has moved from This host to GitHub's plugin page.
- New workflow opens a template gallery: search, categories, a card per template and a preview of what it does before you create it. Templates whose plugin is off are shown faded, PDT's built-in workflows sit in their own group (marked Added or Not added) to add one back, and the Templates list is gone from the Workflows sidebar.
- Settings shows one section per screen, picked from a sidebar with a search box, and every setting saves as you change it (no more Save buttons, except for tokens, raw TOML and the danger zone). New in Settings: where Claude Code, Codex and Devin are installed (Agents), PDT's port and allowed hosts (Network, with Restart now) and the theme (Appearance). The `PDT_*_PROGRAM`, `PDT_PORT` and `PDT_ALLOWED_HOSTS` environment variables still win, and Settings shows them as such.
- `.pdt/project.toml` accepts any forge id: one this PDT has no plugin for is no longer reported as an ignored setting, and the project's origin picks its forge as before.
- CI is no longer tied to the forge: a project's CI can come from GitHub Actions, the local `[ci]` command and Buildkite together, and `[ci] providers` in `.pdt/project.toml` picks which. The pull request's CI badge and automated merges combine them, each provider's failures reach the agent on their own, and `ci_runs` / `ci_failed_logs` name the provider (Buildkite's failed job logs can now be read too).
- Merge… and a ticket's pull request name the project's forge and what it calls its merge queue instead of always saying GitHub, and branch and repository links open on the project's own forge host (a GitHub Enterprise Server too).
- Light mode has more contrast: secondary text is darker and easier to read, off switches, checkboxes, fields and buttons have visible edges, and cards stand out from the page with a faint shadow. Dark mode is unchanged.
- The built-in QA and Coder personas, and the agent guide, tell agents to clean up throwaway servers and scratch folders with commands that need no approval, so agentic QA steps no longer stall on an approval prompt for an `rm -rf` the agent can't judge.

### Fixed

- Open terminals (runs, agent sessions and the shell panel) now recolour straight away when you switch Light, Dark or System, or when the system theme changes, instead of keeping the old background until reopened.
- PDT no longer drops your GitHub login over a single refused call: it asks GitHub again first, and if GitHub later accepts a login it had refused, PDT reconnects on its own within a few minutes. PDT also checks the login every 5 minutes and logs every refusal in detail, to track down GitHub logins that stop working after a few hours.
- Waking the computer from sleep no longer raises a "PDT's engine loop stopped" error in Inbox → Updates: time spent asleep no longer counts as a stalled loop, while a loop that really stops is still reported.
- A PM running on Codex or Devin can report bugs and use its other tools that need your yes. Codex and Devin can't ask in their own prompt, so the call now waits in the Inbox under Approvals: Allow runs it and Deny doesn't, and the PM hears what happened either way.
- Moving a ticket to Done no longer shows its pull request as merged when it isn't: a ticket whose PR is still open (or was closed unmerged) ends its attempt as abandoned instead.

## 0.2.0 (2026-10-29)

### Added

- The Pipeline's Coder now adds a changelog entry for each change when the project keeps a changelog (its agent instructions say how), and the Reviewer checks that the entry is written for the project's users and is accurate.
- A new built-in workflow, "Cut a release" (off by default), lets the PM start a release when you ask for one: it opens the release pull request, puts it in your Inbox for review, and tells you when the release is published.
- The PM can run one of your enabled workflows when you ask, after you confirm it, such as "Cut a release" with the kind of version bump you want.
- PDT runs on Linux (x86_64 and arm64): a headless tarball that `pdt-host install` sets up as a systemd user service, and an AppImage for desktops. Both update themselves like the macOS and Windows apps, and on machines without a system keyring secrets are kept in a file in the data folder that only you can read (Settings → Secrets says which).
- Text from outside PDT (issue titles and bodies, webhooks, imported tickets' descriptions) now reaches agents fenced off as data, never instructions; comments and reviews from people who aren't collaborators on the repository stay on the ticket's timeline instead of going to the agent. Triage is capped per author and per repository an hour (Settings → Host), and the ticket's timeline records each time outside input reached an agent.
- The PM's conversation list puts a dot on a conversation that has replied while you were elsewhere, and a spinner on one that is working; the dot goes once you open it.
- Experimental releases: builds of main published by hand as pre-releases between stable releases. Set Settings → Updates → Update channel to experimental to get them (they can carry database changes no stable release has yet, so such a host stays on them until a newer stable release ships); the update banner and Settings mark them "Experimental". Ask the PM for an experimental release, or run "Cut a release" with channel experimental.

### Changed

- Updates now come only from deliberate releases with SemVer versions (the next is 0.2.0), not from every merge, and an older release is never installed over a newer one. When an update is available, Release notes show the notes of every release between your version and the new one.
- PDT has a new logo, a lowercase "pdt" with a blue underline. It is the browser tab's icon, the app and installer icon, and the tray icon. On macOS the menu bar shows it in black or white to match the bar.
- A new Pipeline accepts tickets labelled `agentic` automatically: once QA passes they merge without waiting for you at Accept. Pipelines you already have keep their own list of labels.
- In a PM conversation, PDT's daily update and its checkpoint and journal requests now show as one muted line ("PDT brought this conversation up to date · 1d ago") that you can open to read in full. The daily update itself is shorter: one line per ticket that changed, no schedule or overlap noise, only what other conversations' journals leave open, and on a quiet day just a checkpoint request or nothing.
- A release you start (or ask the PM for) now ships by itself: its release PR merges once its checks pass, with no review step. "Cut a release" tells you when the PR is open and when the release is out, and puts a failed check in your Inbox with the failed jobs, so you can push a fix or close the PR. To change the release notes, edit the changelog fragments before starting the release.
- Auto-start now takes the most urgent ticket first (urgent, high, medium, low, none), then backlog rank, so a high-priority ticket no longer waits behind a lower one ranked above it. To keep the old order, set the project's "Auto-start picks by" to Backlog rank. To do tickets still come before Ready ones.

### Fixed

- Cancelling a workflow run no longer lets its agent step's session start up again: a nudge PDT had queued for it could reopen the stopped conversation a moment after the cancel.
- Agents can no longer let a ticket from outside PDT (an issue or a report) in by calling PDT's HTTP API themselves: moving it to To do, starting it, labelling it `agentic` or approving the PM's move of it now only works from PDT's own window.
- Opening an Inbox item on a phone no longer leaves a blank screen when PDT was updated since the page loaded: it reloads into the new version. A view that fails to show now says "Something went wrong" with Reload and Back to Inbox instead of a blank page.
- When GitHub stops accepting PDT's login (a revoked or expired token), the Inbox now says "Reconnect GitHub" and Settings › This host › GitHub shows it with a Reconnect button; PDT stops calling GitHub until you connect again, then picks pull requests and CI back up at once.
- Check for updates and Settings no longer say the update token can't be read because macOS "may be waiting for someone to allow access" when there is no prompt: keychain reads no longer fail just because another one is in progress, and PDT remembers what it read instead of asking the keychain on every check. A real keychain timeout now names the secret it was reading.

## 0.1.x: continuous builds

Every merge to main published a release, `v0.1.<build number>`, which the stable app installed when idle or on
Update. Those releases had no changelog: each one's notes were the merged pull request's title and description, and
the app's Release notes listed the tickets each update brought.
