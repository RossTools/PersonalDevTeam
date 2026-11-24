<!-- Title: "<TICKET or GH-<issue>>: what changed", one line: it becomes the commit subject on main. -->

**Ticket:** <!-- e.g. PDT-123, or the issue it fixes ("Fixes #45") -->

## Summary

<!-- What changed and why, for a reviewer. -->

## Checklist

- [ ] A changelog fragment, `.changes/unreleased/<TICKET>.md` (no ticket: `GH-<issue or PR>.md`), written for people who use PDT, or the `no-changelog` label (or `[no-changelog]` in this description) for a PR that only changes docs, CI or tests (docs/SDLC.md "Changelog")
- [ ] Tests cover the change, and `scripts/check.sh` passes locally (what CI runs; `scripts/check.sh --docs` for a docs-only change), with the files `cargo test` generates (`web/src/api/gen`, `site/src/data`) committed
- [ ] The `windows` label, if it changes paths, processes or the file system; `macos` to run the macOS checks on this PR
- [ ] User docs (docs/user) and the developer guide (docs/dev) updated, if behaviour described there changed
