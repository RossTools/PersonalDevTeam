# Changelog

What changed for people who use PDT, newest first. Each section is compiled from the changelog fragments the pull
requests added (`.changes/`; docs/SDLC.md "Changelog") by `scripts/changelog.sh <version>`.

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
