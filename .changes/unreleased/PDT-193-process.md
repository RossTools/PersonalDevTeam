---
kind: internal
---
Every PDT pull request adds a changelog fragment under `.changes/unreleased/` (CI checks it), and `scripts/changelog.sh <version>` compiles them into CHANGELOG.md and the release notes.
