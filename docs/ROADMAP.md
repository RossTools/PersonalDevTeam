# Roadmap

The working list of what comes next, until PDT has its own tickets (Phase 4); then this moves into PDT. Designs live
in docs/superpowers/specs, plans in docs/superpowers/plans, known limits in docs/superpowers/notes/*-carryover.md.

## Done
- Browser favicon: blue terminal prompt matching the UI accent.
- Phase 0: foundation
- Phase 1: terminals (#1)
- Tray-only desktop, CI releases, update from GitHub releases (#2)
- Phase 2: remote host (#3)
- Update incident fixes: keychain never blocks startup, rollback restores the database, signed macOS releases
  (docs/superpowers/notes/update-incident-2026-10-17.md)
- Moved PDT to the Mac mini: it runs the stable PDT that develops PDT (runbook:
  docs/superpowers/plans/2026-10-16-mac-mini-move.md)
- Phase 3a: workspaces and tabs, herdr-style sidebar, Ctrl+] keys, right-click menus (#11)
- Phase 3b: git integration: project keys, clone by URL, GitHub links, worktree adopt/remove, remove projects,
  Files view (#16)

## Now
- **Phase 3c: Design system and shell UX** (spec: docs/superpowers/specs/2026-10-17-design-system-ux-design.md, in
  review; decisions D1–D7 made). 3c-1 (in review): tokens, `web/ui` components (shadcn/ui on React Aria), gallery,
  every screen moved onto them. 3c-2:
  shell fixes (dev badge, view header, ended sessions, settings as a page, command palette, new-tab picker, toasts,
  phone sidebar sheet).

## Next (build order)
The design spec above also fixes the UX and component list of Phases 4–7 (§8); each phase's own spec covers the rest.
1. **Phase 4: Tickets, backlog and board.** Tickets, a backlog list built for many tickets (search, filters, rank,
   bulk actions), a board of committed work, blocker links, areas and overlap detection so agents don't make
   conflicting changes, Start (attempt, worktree, session). This roadmap moves into PDT.
   Own brainstorm and spec.
2. **Phase 5: Structured agents.** Claude and Codex adapters, event log, conversation view, approvals, scratch chats.
3. **Phase 6: Review.** Diff view, line comments, feedback, merge; worktree removal after merge.
4. **Phase 7: Pipeline.** Personas, workflow engine, pdt-mcp, Refine/Review/QA stages, inbox, loop limit.

## Follow-ups (any time)
- **Folder picker:** browse the host's folders to choose a path (Add project → Folder, Clone URL → Into folder, Host
  settings → Projects folder) instead of typing it.
- **Faster CI, next steps** (done: nextest, no debug info, Mac mini runner for PRs, Windows and bundles off the PR
  path; docs/superpowers/notes/ci.md). Next: a single `ci` gate job as the only required check (lets jobs be
  renamed), a Windows runner on the desktop, Dev Drive for hosted Windows.
- **Phone UI** (parallel workstream): single-column layout, touch terminals. The sidebar sheet is in 3c; the
  one-handed inbox in Phase 7 (design spec §6.7, §8.4).
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
- **Developer ID signing + notarization** for macOS releases: CI and scripts are ready; enroll, then replace the
  signing secrets and add the notary ones (docs/superpowers/notes/developer-id.md). Until then every macOS update
  asks for keychain access again.
- Final product name (codename for now; no "ads" in names).
