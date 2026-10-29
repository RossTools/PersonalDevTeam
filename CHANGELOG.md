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

### Changed

- Updates now come only from deliberate releases with SemVer versions (the next is 0.2.0), not from every merge, and an older release is never installed over a newer one. When an update is available, Release notes show the notes of every release between your version and the new one.

### Fixed

- Cancelling a workflow run no longer lets its agent step's session start up again: a nudge PDT had queued for it could reopen the stopped conversation a moment after the cancel.
- Agents can no longer let a ticket from outside PDT (an issue or a report) in by calling PDT's HTTP API themselves: moving it to To do, starting it, labelling it `agentic` or approving the PM's move of it now only works from PDT's own window.
- Opening an Inbox item on a phone no longer leaves a blank screen when PDT was updated since the page loaded: it reloads into the new version. A view that fails to show now says "Something went wrong" with Reload and Back to Inbox instead of a blank page.

## 0.1.x: continuous builds

Every merge to main published a release, `v0.1.<build number>`, which the stable app installed when idle or on
Update. Those releases had no changelog: each one's notes were the merged pull request's title and description, and
the app's Release notes listed the tickets each update brought.
