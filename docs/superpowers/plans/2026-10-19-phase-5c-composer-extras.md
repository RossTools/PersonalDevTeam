# Phase 5c (part 2): the composer's extras

Spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md (§5, §7, §7.2, §14; §13 5c). Part 1 (#52) did
Claude's terminal switch, the cards and diffs; this does slash commands, images, `@` files and the usage /
rate-limit footer.

## Checked first (2026-10-19)

- Claude `-p`: a custom command comes back expanded (`<command-name>/hi</command-name>…`); a local one (`/context`)
  is not echoed at all, so the queued bubble would never clear unless PDT shows the command itself.
- Codex 0.163.0's generated schema: `thread/compact/start`, `review/start {target}`, `localImage` input,
  `account/read` (only the account type is used).

## Tasks

1. pdt-core: `conversation/composer.rs` (image type by bytes, attachment ids, `@` path ranking; TDD); Claude
   `user_line` with image blocks, command echo (`note_sent_command`; expanded replays map back to `/name args`);
   Codex `commands()`, `slash_command`, `user_input`, `billing_label`. Types: `SendMessage.images`,
   `PendingMessage.images`, `AgentCapabilities.terminalCommands`, `ConversationState.billing`.
2. pdt-agents: `LiveConversation::send(&Outgoing)`; Claude reads images as base64 and reports TUI-only commands;
   Codex runs `/compact` and `/review` through their RPCs, sends `localImage` paths and reports the account type.
3. pdt-api: `POST|GET /api/sessions/{id}/attachments…` (raw body, ≤ 10 MB, by bytes), `GET …/files?q=`; messages
   carry image ids (validated against the session), the echoed message row lists them; billing in the state.
4. Web: `@ui` `SuggestionList`, `ImageThumb`; `PromptBox` gains `above`, `attachments`, `onFiles` (paste/drop),
   `canSend`, `completion`; the composer completes `/` and `@` from the keyboard, attaches images (paste, drop, the
   paperclip), and shows billing, context and a rate-limit line (from 80 %; "Rate limited until …").

Spec deviations: uploads are a raw body, not multipart (one image per request, no new dependency); agent commands go
through `…/messages` as text (`/compact`), not a separate `…/command` route: the adapter maps them.
