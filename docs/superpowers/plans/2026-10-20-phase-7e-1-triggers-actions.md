# Phase 7e-1 plan: triggers and actions (PDT-80)

Spec: docs/superpowers/specs/2026-10-20-phase-7e-integrations-design.md §1–§5, §9. One PR, no migration, built in this
order (each step test-first, `cargo test` green before the next). PDT-85 (7e-2) and PDT-86 (7e-3) get their own plans.

1. **Core (pdt-core `workflow/`).** `schedule.rs`: parse `every` durations, cron slots via `croner` + `chrono-tz`
   (latest slot ≤ now, epoch-aligned intervals, zones, DST); unit tests. Event types and payloads in `events.rs`:
   `schedule.fired`, `github.issue.opened`, `pr.commented`, `pr.reviewed`, `webhook.received`, `pdt.notified`.
   Reserved state keys (`_pdt.`) hidden from `{{state}}`.
2. **Trigger plumbing.** Trigger configs as shared fields (flattened) plus the kind's own; `NodeKind::accepts`; the
   engine matcher and the dry run call it. Existing triggers unchanged (test: 7a's built-ins still validate and match).
3. **Schedule.** `nodes/schedule.rs` (config check: exactly one of `every` / `cron`, ≥ 1 min, zone known) and
   `workflow/scheduler.rs` in the engine tick: baseline on first sight or a new version, one event per slot (keyed),
   catch-up of the latest missed slot only. Tests drive the clock.
4. **Webhook.** `hooks.rs`: `POST /api/hooks/{name}` (connection kind `webhook`; bearer or `X-Hub-Signature-256`;
   1 MiB; 401 / 409 / 404; dedupe by delivery header), routed outside the identity guard but inside the Host guard;
   `nodes/webhook.rs` trigger. Tests: good and bad secrets, HMAC, duplicate delivery, remote request without identity
   accepted only with the secret, the trigger matching its hook only.
5. **GitHub.** pdt-github `recent_issues`, `pr_comments`, `pr_reviews` (args + parsers + `FakeGithub`). Sensor
   `github_issues.rs` (repos from enabled workflows and project scopes; baseline in `host_settings`); the PR poller
   reads comments and reviews of open PRs (baseline per attempt) and emits `pr.commented` / `pr.reviewed` with a
   notice. Built-in **PR comments to the agent** (off).
6. **Actions.** `nodes/command.rs` (cwd modes, stdin JSON, timeout kills the tree, parse, quiet, output caps; tests
   with a small script fixture that works on Windows and macOS), `nodes/http.rs` (reqwest; connection auth modes;
   `expect`; fake server), `nodes/discord.rs` (split at 2000, 429 retry; fake server), `nodes/notify.rs`. Dry runs
   report `would`.
7. **Templates.** The three templates of spec §5 as built-in templates in 7d's `workflow/templates.rs`, each
   instantiated, validated and dry-run in a test.
8. **Web.** Notifications follower (toasts; system notification when hidden and allowed; the Settings switch). TS
   types regenerated.
9. **Docs.** docs/reference/workflows.md (kinds, sensors, webhooks, templates, notify), host.md (`/api/hooks`
   exception), agent-guide.md if it lists kinds.
10. **Checks**: fmt, clippy, cargo test, web typecheck/test/build; PR labelled `windows` (process spawning, paths).
