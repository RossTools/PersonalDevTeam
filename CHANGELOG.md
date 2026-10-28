# Changelog

What changed for people who use PDT, newest first. Each section is compiled from the changelog fragments the pull
requests added (`.changes/`; docs/SDLC.md "Changelog") by `scripts/changelog.sh <version>`.

## 0.1.x: continuous builds

Every merge to main published a release, `v0.1.<build number>`, which the stable app installed when idle or on
Update. Those releases had no changelog: each one's notes were the merged pull request's title and description, and
the app's Release notes listed the tickets each update brought.
