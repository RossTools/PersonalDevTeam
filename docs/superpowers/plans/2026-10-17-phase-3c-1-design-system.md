# Phase 3c-1 — Design system library and migration: plan

Spec: docs/superpowers/specs/2026-10-17-design-system-ux-design.md §3–§6, §7 (3c-1), §9 and decisions D1, D4.
Branch `phase-3c-1-ui` (merge main in once PR #17, which adds the spec, has merged).
Mockups: the "PDT UX pass" canvas, component kit artboard (https://claude.ai/artifact/1EKGuHdvGce3SNSFKFgNqY).

**Goal:** a component library in `web/ui` built with shadcn/ui on its React Aria base and themed with our tokens; a
gallery showing every component; every existing screen moved onto it; no silent failures; no `window.confirm`.
Behaviour stays as it is apart from the confirm/toast fixes and one status dot per tab. The shell UX changes
(dev badge, new view header layout, settings page, palette, new-tab picker, phone sheet) are 3c-2.

**Decisions made while planning** (the spec is updated to match):
- **Colour names follow shadcn.** shadcn's components use `bg-muted` (a fill), `text-muted-foreground`,
  `bg-accent` (a hover/selected fill) and `bg-primary`; ours use `text-muted` (text), `bg-accent` (brand blue) and
  `bg-accent-soft`. Keeping both would make every generated component wrong. So the Tailwind colour names become
  shadcn's, all pointing at our `--pdt-*` tokens (tokens.css stays the only place colours are defined), and our
  existing classes are renamed once (Task 4). The status colours (`running`, `waiting`, `failed`, `idle` and
  `*-soft`) keep their names; shadcn has no equivalent.
- **Dark mode stays ours.** The shadcn variables are aliases of `--pdt-*`, which already switch with the system
  setting and `data-theme`, so there is no `.dark` block; Tailwind's `dark:` variant is redefined to match our rule
  (system dark unless `data-theme=light`, or `data-theme=dark`).
- **Where generated code goes:** `web/ui/primitives/` (shadcn output, ours to edit), `web/ui/lib/` (`cn`), `web/ui/hooks/`.
  `web/ui/index.ts` exports only our public components; features import from `@ui`, never from `primitives/`
  directly or from `react-aria-components`/`@base-ui/react`.
- **Touch spike without an iPhone.** This Mac has no Xcode (no iOS Simulator) but has the Android SDK emulator. The
  spike uses the Android emulator (Chrome) and Playwright WebKit with iPhone emulation (`devices['iPhone 15']`,
  `hasTouch`); the real iOS Simulator is optional, if Xcode is installed later.

## Task 1 — touch spike (time-boxed: one day) — done

Result (docs/superpowers/notes/phase-3c-touch-spike.md): keep the React Aria base; style Nova; drag data `text/plain`
with a `pdt:<kind>:<id>` prefix; draggable rows get a `<Button slot="drag">` handle; phone sheets open from a button,
never an edge swipe; drop `next-themes` and the Geist font after init. iOS Safari still to check on a real phone.


Decides between shadcn's React Aria base (D1) and the fallback (Base UI base + dnd-kit) before anything is built on it.

- In a scratch folder (not the repo): `pnpm dlx shadcn@latest init --base aria` in a fresh Vite + React 19 +
  Tailwind 4 app; add `context-menu`, `drawer`, `command`, `popover`, `sonner`, and a two-list drag demo with React
  Aria `GridList` + `useDragAndDrop` (move between lists, as the board will).
- Serve it from the Mac and test:
  - Android emulator (`~/Library/Android/sdk/emulator`), Chrome, at `http://10.0.2.2:<port>`;
  - Playwright WebKit, iPhone emulation, touch: a small script that long-presses, swipes and drags with touch events;
  - the user's phone over Tailscale where available.
- Checks: long press opens the context menu (and doesn't select text or open the browser's menu); a press cancels
  when the finger moves (scrolling a list doesn't click a row); the drawer swipes open from the edge and closed; a card
  drags between lists by touch, and by keyboard; the command list works with the on-screen keyboard up (Android).
- Write the results to `docs/superpowers/notes/phase-3c-touch-spike.md` with the decision. If the aria base fails a
  check that matters, switch to `--base base` + dnd-kit and update the spec's D1; the rest of this plan is unchanged.
- Also record which of shadcn's styles is closest to Graphite's density (24/28px controls, 6px radius); Task 2 inits
  with it.

## Task 2 — set up shadcn in web/ — done

Result: `web/components.json` (aria-nova, aliases `@ui`, `@ui/primitives`, `@ui/lib`, `@ui/hooks`); new dependencies
`react-aria-components`, `class-variance-authority`, `cn`, `tw-animate-css`. The `shadcn` package is **not** a
dependency (its CLI pulls in ~2,500 lockfile lines for one CSS file): its `dist/tailwind.css` is copied to
`web/ui/shadcn.css` and components are added with `pnpm dlx shadcn@latest add <name> -c web`. Geist dropped (Inter
stays). `src/index.css` already aliases shadcn's variables to `--pdt-*` and defines `dark:` by our rule (the Task 3
table); shadcn's `accent` and `muted` colour names stay ours until Task 4. Checks: typecheck, 119 tests, build (JS
unchanged at 978 kB), and before/after screenshots of the demo instance identical apart from its folder path.


- `pnpm dlx shadcn@latest init -b aria -p nova --cwd web` (Nova, from Task 1; `cssVariables: true`, `tsx: true`,
  `rsc: false`, `iconLibrary: lucide`, Tailwind css `src/index.css`, config blank for Tailwind 4).
- `components.json` aliases: `ui` → `@ui/primitives`, `utils` → `@ui/lib/utils`, `hooks` → `@ui/hooks`,
  `components` → `@ui`; tsconfig `paths` and the Vite alias already map `@ui`.
- Dependencies arrive through the CLI (latest versions: `react-aria-components`, `cn`, `sonner`,
  `tailwindcss-react-aria-components` if the base uses it, and whatever else it adds); anything added by hand goes
  through `pnpm add`. Check `pnpm --dir web build` output size before and after; note it in the PR.
- The CLI rewrites `src/index.css`: keep our imports (fonts, tokens.css) and base layer (cursor rules), and replace
  its generated colour values with Task 3's aliases.

## Task 3 — tokens (TDD where it's code)

- `web/ui/tokens.css`: add the spec §4 tokens in both themes: `--pdt-running-soft`, `--pdt-waiting-soft`,
  `--pdt-failed-soft`, `--pdt-diff-add`, `--pdt-diff-del`, `--pdt-diff-add-strong`, `--pdt-diff-del-strong`,
  `--pdt-backdrop`, `--pdt-shadow-popover`, `--pdt-shadow-dialog`, `--pdt-z-*`, `--pdt-control-sm/md/touch`,
  `--pdt-duration`, `--pdt-ease`. Values as on the component kit artboard.
- `src/index.css`: shadcn variables as aliases (no `.dark` block):

  | shadcn | ours |
  |---|---|
  | background / foreground | bg / fg |
  | card, popover, sidebar (+ `-foreground`) | surface / fg |
  | primary / primary-foreground | accent / on-accent |
  | secondary / secondary-foreground, muted | surface-2 / fg |
  | muted-foreground | muted |
  | accent / accent-foreground (hover, selected) | accent-soft / fg |
  | destructive | failed |
  | border, input, sidebar-border | border |
  | ring, sidebar-ring | ring |
  | radius | radius (6px) |

  plus `@theme inline` entries for `running`, `waiting`, `failed`, `idle`, their `-soft`s, diff, `text-2xs` (10px),
  `text-meta` (11px), shadows, and `@custom-variant dark` matching our rule.
- Test (`web/ui/tokens.test.ts`): parse tokens.css; the `@media (prefers-color-scheme: dark)` block and the
  `[data-theme='dark']` block define the same names with the same values; every `--pdt-*` used in index.css exists.

## Task 4 — rename colour classes (mechanical)

- A one-off script (in the PR description, not committed) rewrites `web/src` and `web/ui`: `text-muted` →
  `text-muted-foreground`, `bg-accent-soft` → `bg-accent`, `bg-accent`/`text-accent` (blue) → `bg-primary`/
  `text-primary`, `text-on-accent` → `text-primary-foreground`, `hover:bg-surface-2` → `hover:bg-accent` where it
  marks hover/selection, `bg-surface` → `bg-card`, `text-failed` → `text-destructive` for errors (status `failed`
  stays for status), `text-[10px]` → `text-2xs`, `text-[11px]` → `text-meta`. Review the diff by hand;
  `tokens.css` names don't change.
- Typecheck, tests and a screenshot comparison of the demo instance before/after (no visible change expected).

## Task 5 — web/ui components (TDD: a test per component)

Each one: generated by `shadcn add` where one exists, then edited to Graphite (sizes, density, our variants); a
test for roles, keyboard, focus return and touch where it applies (§9); a gallery entry. Public API as in spec §5.2.

| Component | From | Notes |
|---|---|---|
| `Button`, `IconButton` | button | variants primary/secondary/ghost/danger; sizes sm 24px, md 28px, 44px on `pointer-coarse`; `pending` keeps width + spinner; `IconButton` requires `label` (aria-label + tooltip) |
| `Input`, `Textarea`, `Select`, `Field` | input, textarea, select, field | Field = label + control + hint + error, `inline` layout |
| `Checkbox`, `Switch`, `SegmentedControl` | checkbox, switch, toggle-group | |
| `Kbd`, `Link`, `Spinner`, `Skeleton`, `Tooltip` | kbd, —, spinner, skeleton, tooltip | `Link external` adds the icon and `rel=noreferrer` |
| `StatusDot`, `Badge`, `Chip` | —, badge, — | `StatusDot` keeps its API, adds `pulse` (off under reduced motion) and `ended` (hollow) |
| `Alert`, `Banner`, `EmptyState` | alert, —, empty | `Banner` is UpdateBanner's fixed-height strip |
| `Toaster` + `toast` | sonner | one `Toaster` in AppShell, tokens for colours |
| `Dialog`, `useConfirm` | dialog, alert-dialog | `useConfirm({title, body, action, danger, checkbox?}) → Promise<{ok, checked}>` |
| `Menu` | dropdown-menu, context-menu | one entry model (items, headings, separators, danger, shortcut, submenu); `trigger="contextMenu"` gives right-click + long press; replaces `workspaces/ContextMenu.tsx` and its `bind()` API |
| `Popover`, `Sheet` | popover, drawer/sheet | |
| `CommandList` | command | groups, shortcuts, filtering; used by PlacePicker now, the palette in 3c-2 |
| `ListRow` | — (React Aria GridList row) | leading/title/secondary/trailing, `current`, menu, drag |
| `TreeView` | — (React Aria Tree) | FolderView's tree |
| `ViewHeader`, `Page`, `Section` | — | ViewHeader keeps today's header contents in 3c-1 (3c-2 rearranges them) |

## Task 6 — app components and actions (web/src/common)

- `AgentState` (TDD: a table test of spec §6.5) replaces `ActivityBadge`, `AttentionDots`, `StateDot` and the
  sidebar's `rollup` labels. A tab shows one dot plus the unseen dot.
- `BranchChip`, `ProjectKey`, `InlineRename` (tab rename, workspace rename, new workspace), `PlacePicker` (a
  `CommandList` in a `Popover`; replaces the two selects in the chooser and the Files view).
- `Composer`: the terminal's Compose box moves onto it (same keys and bracketed paste).
- `useAction(fn, {success?})`: runs an async action, exposes `pending`, toasts the API error message on failure.
  Every `void ws.closeTab(…)`-style call in workspaces/, menus and headers goes through it (test: a rejected action
  shows a toast with the message).

## Task 7 — gallery and screenshots

- `web/gallery.html` + `web/src/gallery/main.tsx`: every component and variant, a light/dark/system switch, a
  phone-width frame. Served by `pnpm --dir web dev` at `/gallery.html`; the production build keeps only
  `index.html` as input, so the gallery never ships.
- `web/scripts/ui-screenshots.mjs` (playwright-core, `pnpm add -D`): screenshots of the gallery in light and dark at
  desktop and phone width into a folder given on the command line, for PRs that touch `web/ui` (D4: review only, no CI
  gate). Documents how to install its browser.

## Task 8 — move the screens onto the library

One commit per area; existing tests keep querying by role and name and must pass unchanged except where noted.

- Shell: `IconRail`, `AppShell` empty states (`EmptyState`), `BottomPanel` tabs (`SegmentedControl`-style tabs).
- Workspaces: `Sidebar` (ListRow, InlineRename, Menu), `WorkspaceBar` (tabs keep their custom strip but use
  `AgentState`, `IconButton`, `Menu`), `NewTabChooser` (Dialog/Popover shell, Buttons, PlacePicker; the CommandList
  redesign is 3c-2), `KeyHelp` (Dialog + Kbd), `menus.ts` onto the Menu entry model; delete `ContextMenu.tsx`.
- Projects: `AddProjectDialog`/`AddProjectForm` (Dialog, Field, SegmentedControl), `ProjectPage` (Page, Section,
  Field, useConfirm for remove worktree and remove project, `Link external`).
- Host: `HostSettingsPage` (Page, Section, Field, Switch, Select; checks as status rows), `UpdateBanner` (Banner;
  `window.confirm` → useConfirm).
- Runs: `RunView` (ViewHeader; Reset data → useConfirm), `ProjectSettingsPanel` (Section, Field, CodeViewer for the
  config file), `RunMenu`/`RunBranchForm` (Select), `useStartRun` (`window.confirm` → useConfirm).
- Sessions: `SessionView` (ViewHeader, AgentState; Remove → useConfirm).
- Files: `FolderView` (TreeView, PlacePicker, EmptyState, Skeleton).
- Tests that clicked through `window.confirm` mocks switch to the confirm dialog.

## Task 9 — guard rails

- `web/src/lint.test.ts`, run by `pnpm --dir web test`: fails on `bg-black`, `bg-white`, hex/rgb colours, `text-[…px]`,
  `z-[…]` and `window.confirm` in `web/src` and `web/ui` (except `tokens.css`), and on imports of
  `react-aria-components`, `@base-ui/react` or `@ui/primitives` outside `web/ui`.

## Task 10 — docs and checks

- CLAUDE.md: `web/ui` layout (primitives, lib, public index), "add components with `pnpm dlx shadcn@latest add
  <name> --cwd web`, then theme them", the gallery and screenshot script, the colour names.
- Spec: planning decisions above (colour names, dark variant, folders, spike method); ROADMAP.
- fmt, clippy, `cargo test` (no Rust changes expected), web typecheck/test/build; screenshots of the demo
  instance before/after in the PR. The user tests with Run → pdt on desktop and phone.
