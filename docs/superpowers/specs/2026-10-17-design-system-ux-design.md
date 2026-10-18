# Personal Dev Team — Design System and UX Pass (Phase 3c), and the UX of Phases 4–7

Status: draft, 2026-10-17, awaiting the user's review. Amends the main spec (`2026-10-14-personal-dev-team-design.md`
§7) and the workspaces spec (`2026-10-17-workspaces-design.md`). Mockups: the "PDT UX pass" canvas
(https://claude.ai/artifact/1EKGuHdvGce3SNSFKFgNqY, private to the user).

## 1. Purpose

PDT's interface grew screen by screen through Phases 1–3b. Each screen styles its own inputs, dialogs, menus and
status labels, so the same thing looks and behaves slightly differently in each place, and every new screen copies
the nearest old one. Phases 4–7 add the board, tickets, conversations, diffs, comments, the inbox and personas: four
times the surface of today. Building them on copied class strings would multiply the drift.

This spec does two things:

1. **Phase 3c, a design system and shell pass**: a small component library in `web/ui`, the existing screens moved
   onto it, and the UX problems found in the audit (§2) fixed. Two PRs, before Phase 4.
2. **The UX of Phases 4–7, designed now**: screens, flows, how each fits the workspace/tab model, and the components
   each needs, so each phase starts from an agreed design and adds components to the library instead of inventing
   them. Each phase still gets its own spec for its backend and details; this one fixes how it looks and works.

### Success criteria

- Every form control, dialog, menu, status label, error, empty state and confirmation in the app comes from `web/ui`.
  Feature code contains no raw colour classes outside tokens and no copied control class strings.
- A new screen can be built from library components alone; the gallery shows every component in light and dark.
- A dev instance is visibly a dev instance.
- Phases 4–7 each name the components they add, and none adds a second version of an existing one.

## 2. Audit: what the UI does today

From reading `web/` as of Phase 3b (#16) and screenshots of a demo instance (desktop, phone, light, dark).

**Design system.** `web/ui` holds `tokens.css`, `Button` and `StatusDot`. Everything else is per-file class strings:

| Pattern | Copies today | Drift |
|---|---|---|
| Text field (`field`) | AddProjectForm, ProjectPage, HostSettingsPage, Sidebar (×2), ContextMenu, NewTabChooser, WorkspaceBar rename | `bg-surface` vs `bg-bg`, `px-1`/`px-2`, focus ring on some only |
| Native select | PlacePicker, RunMenu, RunBranchForm, HostSettingsPage (×2), ProjectSettingsPanel | four sizes, two backgrounds |
| Section heading | `text-xs uppercase…` (settings) vs `text-[10px] uppercase…` (sidebar, menus); one renders a file path in upper case | |
| Selected/hover row | IconRail, Settings nav, Sidebar rows, FolderTree, chooser list, BottomPanel tabs, RunView step tabs, AddProjectForm mode tabs | same idea, eight spellings |
| Modal | NewTabChooser (absolute, z-20, `bg-black/20`), KeyHelp (same, centred), AddProjectDialog (portal, z-40, `bg-black/30`) | three focus behaviours; `bg-black/*` is a colour outside tokens.css |
| Confirmation | inline confirm rows (WorktreeRow, RemoveProject), `window.confirm` (RunView reset, useStartRun, UpdateBanner) | two models; the browser dialog can't be styled or tested |
| Error line | `<p role="alert" className="text-xs text-failed">` ×15 | |
| Agent state label | ActivityBadge, Sidebar `rollup`, Sidebar agent row, AttentionDots | tone mapping written four times |
| Unseen dot | three hand-made spans | |
| Small icon button | WorkspaceBar `small`, Sidebar "+", AddProjectDialog close, BottomPanel | |
| External link | `Ext` in ProjectPage; bare "GitHub" anchors in SessionView and RunView | |

Font sizes are arbitrary values (`text-[10px]`, `text-[11px]`); z-indexes are ad hoc (10, 20, 40, 50); there is no
shadow, overlay or control-height token.

**UX.**

1. **Silent failures.** Tab, workspace and menu actions call `void ws.closeTab(…)` etc.; a failed request is an
   unhandled rejection and the user sees nothing.
2. **Status is shown three ways on one tab.** A tab shows the session status dot *and* an activity dot *and* the
   unseen dot, so a working agent has two green dots. The sidebar shows activity instead. One rule is needed.
3. **Ended sessions look broken.** The terminal area is blank with "Session ended." in 11px at the top right; Restart
   is a ghost button in the header next to Remove, which removes with no confirmation.
4. **The view header is mostly path.** The session header spends its width on the full working folder; the branch,
   the useful part, isn't shown as such.
5. **Settings lives in the list column.** The Add project form sits permanently in the 288px sidebar under the
   settings nav; the project page mixes three save models (Save name and key; Save; save on blur) and its config
   heading is a file path in upper case.
6. **Dead navigation.** Inbox, Board and Personas are rail icons that open a "comes in Phase N" note.
7. **No dev/stable distinction.** A dev instance and the stable one look identical. PDT is developed inside PDT, so
   the user often has both open; acting in the wrong one is easy.
8. **Two headings for one list.** The sidebar says SESSIONS and then WORKSPACES; agent rows repeat the project name
   ("Sample App · Sample App") when the agent is in the main checkout.
9. **Phone.** The sidebar stacks under the terminal and takes about half the screen; the work area loses rows.
10. **The new-tab chooser is a form, not a picker.** Two selects, three buttons, a run select, a disclosure and a
    search box; typing filters only the "bring here" list. The main spec's command palette (§7.1) doesn't exist.
11. **Loading is text.** "Loading…" paragraphs, no skeletons; pending buttons only disable.

## 3. Principles

1. **One component per idea.** If two screens show the same idea (a field, a confirm, an agent's state), they use the
   same component. Variants are props, not copies.
2. **Colour means state.** Accent marks selection, focus and the primary action. Green, amber, red and grey mean
   working, needs you, failed and idle, and nothing else. Stages, labels, personas and projects are neutral.
3. **Every action answers.** A click gets a visible result: the change, a pending state on the control, or an error
   next to it (forms) or in a toast (everything else).
4. **The terminal is sacred.** Nothing that appears or disappears may change the terminal's size (the update banner
   rule, generalised): overlays float, strips have a fixed height and exist whether or not they have content.
5. **Keyboard and touch equivalents.** Every drag has a menu item, every menu a long press, every overlay closes on
   Escape and returns focus to what opened it. Touch targets are 44px on coarse pointers.
6. **Dense, calm, quiet chrome.** Small type, few borders, no decorative colour; the content (terminal, diff,
   conversation) gets the space.

## 4. Tokens

Additions to `web/ui/tokens.css` (both themes) and the Tailwind theme in `web/src/index.css`. tokens.css stays the
only place colours live.

| Group | Tokens | Notes |
|---|---|---|
| Status, soft | `--pdt-running-soft`, `--pdt-waiting-soft`, `--pdt-failed-soft` | backgrounds for badges, alerts, banners |
| Diff | `--pdt-diff-add`, `--pdt-diff-del`, `--pdt-diff-add-strong`, `--pdt-diff-del-strong` | Phase 6, defined now so the code viewer and diff share them |
| Overlay | `--pdt-backdrop` | replaces `bg-black/20` and `/30` |
| Elevation | `--pdt-shadow-popover`, `--pdt-shadow-dialog` | menus/popovers vs dialogs/sheets |
| Layers | `--pdt-z-popover` 30, `--pdt-z-dialog` 40, `--pdt-z-menu` 50, `--pdt-z-toast` 60 | |
| Type | `--text-2xs` 10px (labels), `--text-meta` 11px (secondary lines); existing xs 12, sm 14, base 16 | removes `text-[10px]`/`text-[11px]` |
| Control height | `--pdt-control-sm` 24px, `--pdt-control-md` 28px, `--pdt-control-touch` 44px | one height per size across button, input, select |
| Motion | `--pdt-duration` 120ms, `--pdt-ease` | menus, sheets, toasts; none under `prefers-reduced-motion` |

The dark palette is still written twice in tokens.css (media query and `data-theme`); that's required by the
pattern and stays, with a comment and a test that the two blocks match.

## 5. Component library

### 5.1 Structure and rules

- `web/ui` holds **generic** components: no imports from `src/api`, no knowledge of sessions or tickets. Styled only
  with tokens. Each has a test (roles, keyboard, focus) and a gallery entry.
- `web/src/common` holds **app components** shared by features (agent status, branch chip, place picker, composer).
  They compose `web/ui` and may know the API types.
- Features (`workspaces/`, `projects/`, `runs/`, later `tickets/`, `review/`, `inbox/`) compose both and don't style
  controls themselves. A lint check (a grep in the web test step) fails on `bg-black`, raw hex colours and
  `text-[…px]` outside `web/ui`.
- **A library we theme, not one we write** (decision D1): **shadcn/ui on its React Aria base**
  (`shadcn init --base aria`, added July 2026). shadcn copies component source into `web/ui` (we own and edit it),
  styled with Tailwind and CSS variables that we point at tokens.css; features import from `@ui`, never from the
  underlying libraries. Under it, React Aria Components (Adobe, v1.22 as of 2026-10-09) does the behaviour. Touch is
  a hard requirement from the phone work on, and React Aria is the library built for it:
  - one press model for mouse, touch, pen and keyboard (cancelled on scroll or when the finger leaves, no sticky
    hover on touch, no tap delay);
  - long press on any element (tabs, sidebar rows, board cards), with a screen-reader description and a keyboard
    equivalent;
  - drag and drop that works by touch, keyboard and screen reader (the board, tab and workspace moves, §3's rule
    that every drag has an equivalent);
  - comboboxes and lists that behave with the on-screen keyboard (palette, pickers); overlays with scroll locking
    and dismissal that hold up in mobile Safari; focus return everywhere.
  - context menus on right-click and long press (`MenuTrigger trigger="contextMenu"`, v1.20), menus on long press
    (`trigger="longPress"`), and overlays placed around the virtual keyboard (v1.22).
  shadcn's aria base already borrows where React Aria is thin: its Drawer is Base UI's (swipe, snap points,
  virtual keyboard handling) and its toasts are Sonner (React Aria's Toast is still `UNSTABLE_`). React Aria's own
  swipeable `Sheet` arrived in v1.22 and may replace the Base UI drawer once it settles. Not covered: resizable
  panes (`react-resizable-panels`). App gestures (swipe between tabs, the terminal key bar) stay ours.
- **Touch spike first** (done 2026-10-17, passed on Android; findings in docs/superpowers/notes/phase-3c-touch-spike.md: drag data is `text/plain`, draggable rows carry a drag handle, no edge-swipe sheets). The aria base is three months old and React Aria doesn't document how touch drag starts on
  iOS. 3c-1 opens with a short spike in Mobile Safari on the iOS Simulator (Xcode, on the Mac) and Chrome on any
  available Android device or emulator: long-press tab menu, sidebar sheet swipe, dragging a card between two
  lists, the palette with the on-screen keyboard. (No physical iPhone; the Simulator runs the real WebKit.) If the aria base falls short,
  fall back to shadcn's default Base UI base (same component model and styling, so most of the work carries over)
  plus dnd-kit for drag and drop.
- **Gallery**: `web/gallery.html`, a second Vite entry served by `pnpm --dir web dev` and left out of the production
  build. One page, every component in every variant, a theme switch. It is the living spec and the screenshot
  target for reviews.

### 5.2 Inventory

Phase column: when it's needed. **3c** components are built in Phase 3c; the others are built by the phase that
first needs them, to this design.

**Primitives (web/ui)**

| Component | Replaces / does | Phase |
|---|---|---|
| `Button` | adds `danger` variant, `size` sm/md, `pending` (spinner, disabled, keeps width), leading icon | 3c |
| `IconButton` | the four "small" icon buttons; requires `label` (aria-label + tooltip) | 3c |
| `Input`, `Textarea`, `Select` | every field and native select; sizes match Button | 3c |
| `Field` | label + control + hint + error, one layout; `inline` for one-line settings rows | 3c |
| `Checkbox`, `Switch` | remote-access checkbox, skip-permissions toggle | 3c |
| `SegmentedControl` | Folder/Clone URL, Setup/Output, later Split/Unified | 3c |
| `Tabs` | page sub-views (ticket tab, settings project page) — not the workspace tab strip | 4 |
| `Kbd` | key hints in chooser, help, menus | 3c |
| `Link` | internal and `external` (icon, `rel=noreferrer`) | 3c |
| `Spinner`, `Skeleton` | pending buttons; list and page placeholders | 3c |
| `Tooltip` | replaces `title=` on icon buttons and truncated text | 3c |

**Status and feedback (web/ui)**

| Component | Replaces / does | Phase |
|---|---|---|
| `StatusDot` | kept; adds `pulse` for working (off under reduced motion) | 3c |
| `Badge` | neutral or tone (running/waiting/failed/accent); count variant | 3c |
| `Chip` | mono label for keys, branches, labels; optional icon and link | 3c |
| `Alert` | inline error/warning/info with optional action; `role=alert` for errors | 3c |
| `Banner` | the fixed-height strip (UpdateBanner's STRIP), for host-wide notices | 3c |
| `Toast` + `useToast` | errors and results of background actions (§6.3) | 3c |
| `EmptyState` | icon, one sentence, optional primary action | 3c |

**Overlays and navigation (web/ui)**

| Component | Replaces / does | Phase |
|---|---|---|
| `Dialog` | the three modals; title, body, footer actions; focus trap and return | 3c |
| `useConfirm` | `window.confirm` and inline confirm rows; title, consequence text, danger action, optional checkbox ("Also delete the branch") | 3c |
| `Menu` | ContextMenu (pointer, long press) + a button-anchored dropdown (overflow "⋯"); items, headings, separators, danger, shortcut hints, submenus ("Move to workspace ›") | 3c |
| `Popover` | anchored panel (filters, pickers) | 3c |
| `Sheet` | side or bottom panel: ticket peek (4), phone sidebar (3c) | 3c |
| `CommandList` | filterable, keyboard-driven list with groups and shortcuts; the engine of the palette, the new-tab chooser, "Move to workspace…", branch and persona pickers | 3c |
| `ListRow` | the selected/hover row: leading slot, title, secondary line, trailing slot, `current`, drag handle; used by sidebar, settings nav, chooser, inbox, file tree | 3c |
| `TreeView` | folder view, diff file list, collapsible groups; keyboard per the ARIA tree pattern | 3c (folder view), 6 |
| `Toolbar` / `ViewHeader` | the header of every view: status, title, chips, meta, primary action, overflow menu; wraps to two lines on phone | 3c |
| `Page`, `Section` | settings-style pages: max width, section heading (`text-2xs` label), description, rows | 3c |
| `SplitPane` | resizable two-pane layout (review, inbox, settings on desktop) | 6 |

**App components (web/src/common)**

| Component | Does | Phase |
|---|---|---|
| `AgentState` | the one mapping from (status, activity, unseen) to dot, label and tone; used by tabs, sidebar, header, board cards, palette | 3c |
| `BranchChip` | branch name, GitHub tree link, "Open PR" in its menu | 3c |
| `ProjectKey`, `TicketKey` | `PDT`, `PDT-42`; ticket key links to the ticket | 3c / 4 |
| `PlacePicker` | project + checkout as one `CommandList` popover instead of two selects | 3c |
| `InlineRename` | tab rename, workspace rename, new workspace name | 3c |
| `Composer` | multi-line prompt box: Enter/Shift+Enter rules, send, pending; the terminal Compose today, and the conversation, review feedback, inbox answers and ticket comments later | 3c (terminal Compose), 5 |
| `Markdown` | sanitised GitHub-flavoured markdown, code blocks highlighted with the CodeViewer palette | 4 |
| `CodeViewer`, `DiffViewer` | read-only code (exists); diff with gutter comments | 3b, 6 |
| `Timeline` | ordered events with actor, time and body: ticket activity, outcomes, persona versions | 4 |
| `StageStepper` | the workflow stages with the current one, loop count, owner | 4 (stages), 7 (owners) |
| `PersonaBadge` | persona name with role initial; neutral colour | 7 |

## 6. Interaction patterns

### 6.1 Saving

- A control that is itself the whole decision (switch, checkbox, select, radio) **saves immediately** and shows
  "Saved" next to it for two seconds, or its error.
- Text belongs to a **form**: a Save button appears enabled only when the form is dirty; Enter submits; leaving the
  page with unsaved text asks first. No save-on-blur.
- One Save per section, never per field (fixes "Save owner", "Save ports", "Save name and key").

### 6.2 Destructive actions

- Reversible actions (close tab, stop a run, move a tab) never confirm.
- Irreversible ones confirm with `useConfirm`, which says what is lost: remove project, remove worktree (with the
  dirty-changes warning and the delete-branch checkbox), reset run data, remove an ended session, merge (Phase 6).
  The confirming button uses the `danger` variant and names the action ("Remove worktree", never "OK").
- Stopping an agent that is working confirms; stopping an idle one doesn't.

### 6.3 Errors

- Form submit errors: an `Alert` under the form, focus moves to it.
- Everything else (tab, menu, drag, header actions): a `Toast` with the API's message and, when it helps, an action
  ("Retry", "Open settings"). Every async action handler goes through one `useAction` helper that pending-states the
  control and toasts on failure, so nothing is dropped with `void` again.
- Page load errors: an `EmptyState` with the message and Retry.

### 6.4 Loading and empty

- Lists and trees show `Skeleton` rows (three) while loading; pages show skeleton blocks; nothing shows "Loading…"
  for less than 300ms (no flash).
- Every list has an `EmptyState` with the next step as its action ("Start an agent", "Add a project", "New ticket").

### 6.5 Status vocabulary

`AgentState` is the only place this is decided:

| Situation | Dot | Label |
|---|---|---|
| Agent working | green, pulsing | Working |
| Agent needs you (permission, question, approval) | amber | Needs you |
| Agent idle and not yet seen since it finished | grey + accent unseen dot | Done |
| Agent idle, seen | grey | Idle |
| Session ended or interrupted | hollow grey | Ended / Interrupted |
| Failed (run or session) | red | Failed |
| Shell running | green, no pulse | — |

A tab shows **one** dot (the row above) plus the unseen dot; labels appear where there's room (sidebar, headers,
cards). Workspaces roll up the most urgent of their tabs, as today.

### 6.6 Keyboard

The Ctrl+] prefix keys stay. Additions: `k` opens the command palette (also ⌘K on macOS, which the terminal never
receives); `i` inbox, `l` backlog, `b` board (from their phases); `/` focuses the filter of the current list. Every overlay:
Escape closes, focus returns to the opener. Lists: Up/Down, Enter, Home/End.

### 6.7 Phone

- The sidebar becomes a `Sheet` from the left, opened by the workspace name in the header and closed by a swipe or
  a tap outside; the work area keeps the full height. No edge swipe to open it: the left-edge swipe is the system or
  browser Back gesture on Android and iOS (touch spike, docs/superpowers/notes/phase-3c-touch-spike.md).
- The rail stays at the bottom (as today) with at most five items; Settings moves into the sheet's footer.
- Inbox and approvals are the phone's main job from Phase 7 (§8.4): designed for one hand.

## 7. Phase 3c: what changes on screen

Two PRs, behaviour otherwise unchanged:

**3c-1 — Library and migration.** Tokens (§4), the 3c components (§5.2), the gallery, the lint check; every existing
screen moved onto them. Screens look the same or tidier; no flow changes except the `useConfirm`/toast fixes.

**3c-2 — Shell UX.** The fixes from the audit:

1. **Instance identity.** `/api/health` gains `profile` and `port`. A non-stable instance shows a `DEV · 8512` badge
   at the top of the rail (amber outline, not the status amber fill), and the browser title gets a `[dev]` prefix.
2. **Rail.** Shows only views that exist: Sessions, Files, Settings in 3c; Tickets (backlog and board) joins in 4, Inbox and Personas in 7.
3. **View header** (`ViewHeader`): state dot, title, `BranchChip` (or the project key for the main checkout), the
   agent label, then the primary action and a "⋯" menu (Compose, Restart, Copy path, Open on GitHub, Move to
   workspace, Remove…). The folder path moves into the menu and the title's tooltip.
4. **Ended sessions.** The terminal keeps its last screen, dimmed, under a fixed-height bar: "Session ended ·
   Restart · Remove…" (Restart primary). Interrupted sessions get "Resume" with the existing explanation.
5. **Sidebar.** One heading per section (drop the duplicate view label in the Sessions view); agent rows show the
   branch (or nothing for the main checkout) and the workspace only when it isn't the current one.
6. **Settings as a page.** The list column holds the settings nav (This host, projects with keys, "Add project" →
   the dialog); the form leaves the sidebar. Project page sections: Project (name, key, folder, GitHub), Worktrees,
   Runs on this host (config file in a `CodeViewer`, overrides, credentials), Danger zone (remove). Host page:
   Remote access, Updates, Runs, Agents, Projects folder, Checks (as status rows with `AgentState`-style dots).
7. **Command palette** (`k`, ⌘K): go to any workspace, tab, agent, project or setting; run any action (new
   Claude/Codex/shell here, run…, rename tab, move tab, toggle theme). Results are grouped; typing filters across all.
8. **New-tab chooser** becomes a `CommandList` in a popover under "+": the place as a `PlacePicker` chip at the top,
   then Claude `c`, Codex `x`, Shell `s`, the configured runs and tests, "Run another branch…", and below, matching
   sessions and runs to bring here. One filter box for all of it; the skip-permissions switch stays in the footer.
9. **Toasts** for every background action error (§6.3).
10. **Phone sidebar** as a sheet (§6.7).

## 8. Phases 4–7: UX design

### 8.0 How new things fit workspaces and tabs

A tab today shows a session or a run. Phases 4–7 add two tab kinds; the rules from the workspaces spec stay (an item
has at most one tab; closing a tab never stops anything; launches open tabs in the project workspace unless placed).

| Tab kind | Shows | Added in |
|---|---|---|
| session | terminal (shell, interactive agent) or conversation (structured agent) | 1 / 5 |
| run | run output and controls | 2 |
| **ticket** | the ticket hub: Overview · Conversation · Changes · Files | 4 (Overview), 5, 6 |
| **changes** | review of a branch that has no ticket | 6 |

- **Structured pipeline sessions don't get their own tabs**; they live in their ticket's tab (Conversation lists them
  by stage and persona). Their state rolls up into the ticket tab's dot, so a reviewer that needs you lights the
  ticket's tab and workspace. Interactive terminals and shells for a ticket do get tabs, placed right after the
  ticket tab.
- **Tickets (backlog and board), Inbox and Personas are views** (rail icons), not tabs, like Settings and Files: they're places you go to
  pick work, and opening a ticket from them opens or focuses its tab.
- Storage: `workspace_tabs` gains `ticket_id` (Phase 4) and `worktree_id` (Phase 6, for changes tabs) with the
  exactly-one CHECK extended; a new migration each time.

### 8.1 Phase 4 — Tickets and board

**Tickets view** (rail: Tickets) has two sub-views, picked with a `SegmentedControl` in its header and remembered
per browser: **Backlog** (Ctrl+] l) and **Board** (Ctrl+] b). Both share the list column (projects with keys and
open-ticket counts, "All projects", saved filters) and the same filters, so switching keeps what you're looking at.

**Backlog** — every ticket, built for hundreds to thousands of them

- **Who it's for:** planning. The backlog is where tickets are written, ranked, groomed and pulled into work; the
  board shows only work that is committed.
- **Layout:** a dense list (React Aria `GridList` or `Table` with its virtualiser, so thousands of rows scroll
  smoothly): rank handle, key, title, stage, priority, labels, attempt state (`AgentState` when started), updated.
  Columns can be shown or hidden; on a phone each row is two lines (key + title, then stage · priority · labels).
- **Order:** manual rank by default (drag, or Alt+↑/↓, or the row menu's "Move to top / bottom"); sorting by
  priority, updated or created is a view option that doesn't change the rank.
- **Group by:** none (default), stage, priority or label, with collapsible group headers and counts.
- **Filter bar:** text search over key, title and description (server-side), then chips for stage, priority, label,
  "Has attempt" and "Needs you". Done and abandoned tickets are hidden unless "Show closed" is on. Filters can be
  saved by name into the list column. `/` focuses search.
- **Bulk actions:** checkbox per row, Shift-click for ranges, Cmd/Ctrl+A; a fixed-height action bar appears in place
  of the filter bar's secondary row (never moving the list): Move to Ready, Set priority, Add or remove label, Move
  to top, Delete…. On touch, long press starts selection mode.
- **Quick add:** an inline row at the top ("New ticket title… Enter to add, Shift+Enter for details"); new tickets
  land in Draft at the top of the rank.
- **Open:** clicking a row opens the same peek `Sheet` as the board; Enter or "Open" opens the ticket tab.
- **Scale:** the list asks pdtd for one page at a time (`GET /api/tickets?project=&q=&stage=&label=&sort=&cursor=`)
  and never loads every ticket; rank is a fractional index (a sortable string), so moving a ticket updates one row.
  Phase 4's own spec settles the API.

**Board** — committed work only

- Columns (decision D3): **Ready**, **In progress**, **Done** (last two weeks, then "Show more"). Draft tickets live in
  the backlog and reach the board by moving to Ready ("Move to Ready" in the backlog, or dragging a backlog row onto
  the Board segment). Phase 7 splits In progress into Refine, Code, Review, QA and Accept once the pipeline moves
  tickets itself.
- The user moves cards (drag, the card menu's "Move to ›", or `[`/`]` on a focused card). Within a column cards
  follow the backlog's rank, so ranking in one view ranks in both.
- A WIP hint per column (count, and amber when In progress has more tickets than running agents can serve) rather
  than a hard limit.
- **Card**: ticket key, title (two lines max), priority icon, labels as neutral chips, and when it has an attempt:
  the branch and the attempt's most urgent `AgentState` dot. Unseen activity bolds the title, as in the sidebar.
- **Quick add** at the top of Ready, same as the backlog's.
- **Peek**: clicking a card opens a `Sheet` on the right with the ticket (Overview content) and its actions;
  Enter or "Open" opens the ticket tab. The board stays visible beside the sheet on desktop; full-screen on phone.

**Ticket tab — Overview**

- Header (`ViewHeader`): `TicketKey`, title (inline edit), `StageStepper`, priority, labels, `BranchChip` when an
  attempt exists; primary action by stage: Draft → "Submit" (to Ready in Phase 4, Refine in 7), Ready → "Start",
  in progress → "Open terminal"; "⋯" for Move to stage, Abandon attempt, Delete ticket.
- Body: description (`Markdown`, Edit switches to a `Textarea` with preview), then a `Timeline` of activity (created,
  moved, started, sessions, later outcomes and comments) with a comment `Composer` at the bottom.

**Start** (the Ready → Code step)

- A `Dialog`: agent (Claude / Codex), branch (prefilled `pdt/PDT-42-<slug>`, editable), base branch, skip-permissions
  switch, "Open in a new workspace" checkbox (off: the project workspace). Start creates the attempt, worktree and
  an interactive agent session primed with the ticket (title + description as its first prompt), then opens the
  ticket tab and the session tab after it.
- Phase 5 adds "Structured (in the ticket)" vs "Terminal" as the agent mode; Phase 7 replaces the agent choice with
  the stage's persona.

**Links and blockers**

- A ticket can link to others as **blocked by** / **blocks** (one link, shown from both ends) or **related**. Links
  are added from the ticket's Overview ("Link ticket…" opens a `CommandList` picker searching key and title; pick
  the kind first, `b` blocked by, `l` blocks, `r` related), from the row or card menu, or by typing `blocked by
  PDT-12` in a comment's first line. Links across projects on the same host are allowed. A link that would make a
  cycle of blockers is refused with the cycle named.
- A ticket is **blocked** while any blocker isn't Done. Blocked tickets show a lock `Badge` ("Blocked by PDT-12"):
  backlog rows, board cards, the ticket header, palette results. Filters gain "Blocked" and "Unblocked", and the
  backlog ships a saved filter **Ready to start**: Ready, unblocked, and no conflict (below).
- When a blocker reaches Done, each ticket it blocked gets a timeline entry and, if it is now unblocked, a toast to
  anyone looking at it ("PDT-42 is unblocked"); Phase 7 turns this into an inbox item or an automatic start.
- The Overview's side column lists links by kind with each ticket's stage and `AgentState`; the board peek shows the
  same in short form.

**Avoiding conflicting work**

Two agents changing the same code at the same time produce conflicts (in this repo: two branches that each add
migration `0010_*.sql`, or both regenerate `web/src/api/gen`). PDT prevents what it can and shows the rest:

- **Areas** are named parts of a project, declared in `.pdt/project.toml` (host overrides allowed, like runs):

  ```toml
  [[area]]
  name = "migrations"
  paths = ["crates/pdt-store/migrations/**"]
  exclusive = true        # only one active ticket at a time
  [[area]]
  name = "web/ui"
  paths = ["web/ui/**"]
  ```

  A ticket lists the areas it expects to touch (chips on the Overview and in the full ticket form; Phase 7's Refiner
  proposes them). A ticket with no areas declared is treated as touching nothing declared, never as touching
  everything.
- **Planned overlap:** two active tickets (an attempt that isn't merged or abandoned) that share an area overlap. In
  an **exclusive** area that's a conflict; in a normal area it's a warning.
- **Actual overlap:** pdtd works out each active attempt's changed files (`git diff --name-only` against the merge
  base, plus uncommitted changes) when the attempt's worktree changes, and flags any file changed by two active
  attempts, declared or not. Files matching an exclusive area count as a conflict.
- **Where it shows:** an amber "Overlaps PDT-51" `Badge` (red "Conflicts" for exclusive areas or the same file) on
  cards, rows and the ticket header; the Overview's side column lists each overlap with the shared area or files and
  the other ticket's stage and agent state. Filter chip "Conflicts".
- **Start checks first.** The Start dialog shows an `Alert` for each blocker that isn't Done and each planned
  conflict, then offers **Start anyway** (secondary, the `danger` variant for exclusive conflicts) or **Wait**
  (primary): the ticket is marked *waiting* with its reasons, sits at the top of Ready with a clock badge, and gets
  an "is clear to start" toast (Phase 7: an inbox item or automatic start) when the last reason clears.
- **While working:** when actual overlap appears between two running attempts, both tickets' agents keep going, but
  their tabs and cards show the badge and the ticket timeline records which files; the user decides (pause one, or
  let them run and merge one first). Phase 6's Merge dialog lists the other active attempts that touch the merged
  files, since they will need a rebase.
- **Phase 7** makes this automatic: the pipeline never auto-starts a blocked ticket or one with a planned conflict,
  and starts waiting tickets in rank order as they clear.

**Elsewhere**: ticket keys appear in sidebar agent rows and tab titles (`PDT-42 · Claude`); the palette searches
tickets by key and title; the new-tab chooser gains "Ticket…" to open one here.

Components added: `Tabs`, `Markdown`, `Timeline`, `StageStepper` (stages only), `TicketKey`, `TicketPicker`
(`CommandList` over tickets, for links and "Ticket…"), `LinkList`, `AreaChips`, `OverlapBadge`, `DataList` (virtualised
list/table with selection, rank drag, grouping and column options), `FilterBar` (search + filter chips + saved
filters), `BulkActionBar`, board `Column`/`Card` (feature-level), drag between columns via React Aria.

### 8.2 Phase 5 — Structured agents

**Conversation** (a structured session's tab, or the ticket tab's Conversation)

- A message list: the user's messages right-aligned in a `surface-2` bubble; the agent's as plain `Markdown` on the
  page (no bubble); tool calls as compact `ToolCall` rows ("Edit `src/app.ts` +12 −3", "Run `cargo test`") that
  expand to their input and output (diffs in `DiffViewer`'s inline mode, output in a terminal-styled block).
- **Approvals** are `ApprovalCard`s in the stream, amber edge, with what is asked (command or diff), and Approve,
  Deny, "Always allow this tool in this session"; keys `y`/`n` when focused. The session's state is Needs you while
  one is open. The same card appears in the inbox (Phase 7).
- Footer: the `Composer` (Enter sends, Shift+Enter new line), Interrupt while working, the model and token usage
  quietly at the right. A "Jump to latest" pill when scrolled up while new events arrive.
- In a ticket tab, a session switcher above the stream lists its sessions ("Coder · Claude", "Reviewer · pass 2"),
  newest first, with their `AgentState`.
- Unknown events render as a collapsed "Unrecognised event" row with the raw JSON (the main spec's drift rule).

**Scratch chats**: the new-tab chooser gains "Chat (Claude)" / "Chat (Codex)": a structured session in the current
place, no ticket. Its header's "⋯" has "Make ticket…", which opens the ticket form prefilled with a title and summary
the agent drafts from the conversation.

Components added: `ToolCall`, `ApprovalCard`, `MessageList` (virtualised, sticky-bottom), the `Composer` grows
attachments later.

### 8.3 Phase 6 — Review

**Changes** (the ticket tab's Changes, or a changes tab for any worktree from the project page, a `BranchChip`
menu or the palette)

- `SplitPane`: left, the changed files as a `TreeView` (status letter, +/− counts, comment count; "Viewed" checkbox
  per file); right, the `DiffViewer` for the selected file, Split/Unified as a `SegmentedControl` (unified is the
  default under 900px), whole-file context on demand.
- **Comments**: hover a line number for "+", or select lines and press `c`; the thread opens inline under the line
  with a `Composer`. New comments are drafts (dashed outline, "Draft" badge). Agent reviewers' comments (Phase 7) show
  their `PersonaBadge`. Threads have Resolve.
- **Review bar** (fixed height, top of the diff): "3 draft comments" → **Send feedback** (primary; sends to the
  coder session and marks them sent), **Approve**, and in Accept, **Merge…**.
- **Merge…** is a `useConfirm` dialog: into which base branch, how many commits, "Remove the worktree" (on) and
  "Delete the branch" (off) checkboxes; uncommitted changes or a conflict show an `Alert` instead of the button.
- GitHub PR state (when the GitHub integration follow-up lands) shows as a `Badge` in the review bar.

Components added: `SplitPane`, `DiffViewer`, `CommentThread`, `Comment`, file `TreeView` mode.

### 8.4 Phase 7 — Pipeline

**Inbox** (rail: Inbox, with a count `Badge`; Ctrl+] i)

- List column: inbox items grouped by kind, most urgent first: Questions (clarifications), Approvals, Stalled, Loop
  limit, Ready to accept. Each row: `TicketKey`, title, kind, age, persona.
- Work area: the item and its decision, with just enough context (the ticket's summary, the last messages or the
  diff stat), and a link to the full ticket tab:
  - Question: the agent's questions and a `Composer` per question (or one for all); Send answers.
  - Approval: the `ApprovalCard`.
  - Stalled: Nudge (sends a reminder) / Take over (makes the stage manual).
  - Loop limit: the loop history; Send back to Code once more / Take over / Abandon.
  - Accept: the review summary; Open changes / Merge… / Reject (with a reason → Code).
- Triage keys: `j`/`k` move, Enter acts on the primary action, `o` opens the ticket, `e` marks done where that makes
  sense. Resolving moves to the next item.
- **Phone**: the inbox is the default view on a phone once it exists: one item per screen, the decision buttons in
  the thumb zone, swipe for next.

**Board**: cards gain the owner's `PersonaBadge` and the stage session's live `AgentState`; In progress splits into
the pipeline stages (Refine, Code, Review, QA, Accept); manual moves are limited to what the workflow allows (other moves ask "Override the workflow?").

**Pipeline scheduling**: the engine starts Ready tickets in backlog rank order, skipping blocked ones and ones with a
planned conflict (§8.1); waiting tickets start as their reasons clear. A project setting caps how many attempts run
at once. The board's In progress column shows why each Ready ticket at its top hasn't started.

**Ticket tab**: the `StageStepper` shows owners per stage and `loop 2/3`; the Overview timeline shows outcomes with
their summaries; the Conversation lists every stage's sessions.

**Personas view** (rail: Personas): list of personas (role, current version's agent and model); the detail is a form
(agent, model, instructions in a `Textarea` with markdown preview, permission profile) where Save makes a new version,
and a `Timeline` of versions with a diff between any two (`DiffViewer`). Sessions link to the version they ran.

Components added: `PersonaBadge`, inbox item layouts (feature-level), `StageStepper` owners.

## 9. Testing

- Every `web/ui` component: Vitest + Testing Library for roles, keyboard (open, move, select, Escape), focus return
  and the touch paths: long press opens the same menu as right-click, a touch press cancelled by moving doesn't
  fire, and drag has its keyboard equivalent. `AgentState`: a table test of §6.5.
- Touch is checked by hand before a PR that changes touch behaviour merges: Mobile Safari in the iOS Simulator and
  Chrome on Android (device or emulator), against the branch's Run; jsdom can't reproduce mobile Safari.
- The lint check (§5.1) runs in `pnpm --dir web test`.
- Screens moved in 3c-1 keep their existing tests passing unchanged where they query by role and name; tests that
  relied on `window.confirm` switch to the confirm dialog.
- Gallery screenshots (light and dark, desktop and phone) are taken by a script for review in PRs that touch
  `web/ui`; not a CI gate yet (decision D4).

## 10. Decisions

- **D1. UI library — decided 2026-10-17:** shadcn/ui with the React Aria base, themed with our
  tokens, after the touch spike (§5.1), because mobile touch will be a hard requirement. Considered:
  - shadcn/ui on Base UI (shadcn's default since July 2026; Base UI v1.9, stable since December 2025): the
    mainstream path, with a stable swipe Drawer and long-press ContextMenu, but no drag and drop, no collection or
    virtualised list components, and less thorough press handling. The fallback if the spike fails.
  - shadcn/ui on Radix: maintained (WorkOS) but slower-moving; shadcn no longer defaults to it.
  - React Aria's own Tailwind starter without shadcn: same behaviour, fewer finished pieces.
  - Ark UI (Zag.js): multi-framework, no evidence of a touch advantage; we're React-only.
  - MUI/Mantine/Chakra: own design language and theme system, duplicate tokens.css, fight Tailwind and our density.
  - Hand-rolling: cheapest now, costliest by the board and the phone.
- **D2. Timing — decided:** Phase 3c comes before Phase 4, as two PRs (3c-1 library and migration, 3c-2 shell UX).
- **D3. Board columns before the pipeline — decided:** simple columns until Phase 7. With the backlog view (§8.1)
  holding Draft, the board's columns are Ready, In progress and Done; Phase 7 splits In progress into the stages.
- **D4. Visual checks — decided:** gallery screenshots (light/dark, desktop/phone) for review in PRs that touch
  `web/ui`; not a CI gate yet.
- **D5. Where a started ticket lives — decided:** a ticket is a tab in its project workspace, its terminals as tabs
  after it; Start has "Open in a new workspace" for big tickets.
- **D6. Backlog as its own view — decided (user, 2026-10-17):** the number of tickets will grow large, so the
  backlog is a separate, virtualised, filterable list (§8.1), and the board shows only committed work.
- **D7. Blockers and conflicting work — decided (user, 2026-10-17):** ticket links (blocked by / blocks, related)
  and conflict prevention through declared areas (some exclusive) plus detected file overlap between active
  attempts, built in Phase 4; Phase 7 enforces them when it starts work automatically.


## 11. Deferred

- A theme editor or user-chosen accent; density settings.
- A settings toggle to keep the not-yet-built rail views visible.
- Notifications (system/push) for needs-you — the Notifications follow-up; this spec's toasts are in-app only.
- Board swimlanes, epics, sprints or milestones.
- Inferring areas from history (files a ticket's earlier attempts touched) or from the code graph.
- Editing workflows in the UI (main spec: out of Milestone 1).
