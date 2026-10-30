<!-- Title: "<TICKET>: what changed", one line: it becomes the commit subject on main. -->

**Ticket:** <!-- e.g. PDT-123, or the issue it fixes ("Fixes #45") -->

## Summary

<!-- What changed and why, for a reviewer. -->

## Checklist

- [ ] A changelog fragment, `.changes/unreleased/<TICKET>.md`, written for people who use PDT, or the `no-changelog` label for a PR that only changes docs, CI or tests (docs/SDLC.md "Changelog")
- [ ] Tests cover the change, and the checks pass locally: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (generated types in `web/src/api/gen` committed), `pnpm --dir web typecheck`, `test` and `build`
- [ ] The `windows` label, if it changes paths, processes or the file system
- [ ] User docs (docs/user) and the developer guide (docs/dev) updated, if behaviour described there changed
