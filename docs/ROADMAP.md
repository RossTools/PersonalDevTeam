# Roadmap

The working list of what comes next, until PDT has its own tickets (Phase 4); then this moves into PDT. Designs live
in docs/superpowers/specs, plans in docs/superpowers/plans, known limits in docs/superpowers/notes/*-carryover.md.

## Done
- Phase 0: foundation
- Phase 1: terminals (#1)
- Tray-only desktop, CI releases, update from GitHub releases (#2)
- Phase 2: remote host (#3)
- Update incident fixes: keychain never blocks startup, rollback restores the database, signed macOS releases
  (docs/superpowers/notes/update-incident-2026-10-17.md)
- Moved PDT to the Mac mini: it runs the stable PDT that develops PDT (runbook:
  docs/superpowers/plans/2026-10-16-mac-mini-move.md)

## Now
- **Phase 3: Workspaces** (spec: docs/superpowers/specs/2026-10-17-workspaces-design.md). Two PRs: 3a workspaces
  and tabs (no split panes), session tree, keyboard, status at a glance, drag to reorder; 3b project keys, clone by URL,
  GitHub links, adopt/remove worktrees, remove projects, folder view.

## Next (build order)
1. **Phase 4: Tickets and board.** Tickets, board, Start (attempt, worktree, session). This roadmap moves into PDT.
2. **Phase 5: Structured agents.** Claude and Codex adapters, event log, conversation view, approvals, scratch chats.
3. **Phase 6: Review.** Diff view, line comments, feedback, merge; worktree removal after merge.
4. **Phase 7: Pipeline.** Personas, workflow engine, pdt-mcp, Refine/Review/QA stages, inbox, loop limit.

## Follow-ups (any time)
- **Faster CI, next steps** (done: nextest, no debug info, Mac mini runner for PRs, Windows and bundles off the PR
  path; docs/superpowers/notes/ci.md). Next: a single `ci` gate job as the only required check (lets jobs be
  renamed), a Windows runner on the desktop, Dev Drive for hosted Windows.
- **Phone UI** (parallel workstream): single-column layout, touch terminals.
- **Notifications** for needs-you and unseen sessions.
- **Work laptop install:** separate self-contained install, corporate accounts, no Tailscale, updates by token or by
  hand. Needs remote-access-free use to be smooth.
- **Devin** on the work laptop: a terminal session type first, an adapter later.
- **Replace hermes-agent** on the Mac mini: own brainstorm, after the agentic phases.
- **GitHub integration / updater revisit** alongside Phase 3 git integration.
- **Automatic per-PR runs**, a **multi-host UI**, **device tokens** (remote access without Tailscale).
- **Remote-access run restart gaps** (docs/superpowers/notes/phase-2-carryover.md).
- **Flaky test watch:** `hook_events_change_a_sessions_activity` failed once on Windows CI (empty activity); it now
  prints the response body.
- **Developer ID signing + notarization** for macOS releases (Gatekeeper); the self-signed certificate only keeps
  the keychain's trust across updates.
- **Host settings with an unreadable keychain:** the page fails with `keychain_unavailable`; show settings and the
  token problem instead.
- Final product name (codename for now; no "ads" in names).
