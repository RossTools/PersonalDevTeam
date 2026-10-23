# Phase 3c touch spike (2026-10-17)

Plan: docs/superpowers/plans/2026-10-17-phase-3c-1-design-system.md, Task 1. Question: does shadcn/ui on its React
Aria base (decision D1) hold up on a phone before we build on it?

**Decision: keep the React Aria base.** Every check passed on Android Chrome with real touch input. Two findings change
the design (below). iOS Safari is still unverified.

## Setup

- Scratch Vite + React app (outside the repo): `shadcn init -b aria -p nova` (shadcn 4.22.0), then `shadcn add button
  context-menu drawer command popover sonner`. Installed: `react-aria-components` 1.22.1, `@base-ui/react` 1.9.0 (the
  drawer), `sonner`, `cn`, `class-variance-authority`, `tw-animate-css`, `next-themes` (Sonner's theme hook; we won't
  need it), `@fontsource-variable/geist` (Nova's font; we keep Inter).
- Device: the Mac's Android emulator (Pixel 8, Android 36, Chrome), page served from the Mac through `adb reverse`.
  Input through `adb shell input` (`tap`, `swipe`, `motionevent DOWN/MOVE/UP`), which the system treats as real touch.
- No iOS: this Mac has no Xcode (so no iOS Simulator) and there's no iPhone.

## Results (Android Chrome, real touch)

| # | Check | Result |
|---|---|---|
| 1 | Tap a tab fires its press; long press opens the context menu (`MenuTrigger trigger="contextMenu"`), with no press, no text selection, no Chrome menu; a menu item fires | Pass |
| 2 | Scrolling a list (two swipes) fires no row action; a tap on a row does | Pass |
| 3 | Drawer from the left (Base UI via shadcn): opens from a button, swipes closed | Pass |
| 3b | Edge swipe from the left to open the drawer | **Fails by design**: Android's system Back gesture takes it (the browser went back). iOS Safari's edge swipe is Back too |
| 4 | Long press then drag a card between two React Aria `GridList`s (`useDragAndDrop`, `onInsert`) | Pass, once the drag data is `text/plain` (below) |
| 4b | Same with a custom drag type (`pdt-card`) | **Fails**: the drag starts, no drop ever lands. A plain HTML drag-and-drop control on the same page worked, so it's the custom type on Android Chrome, not injected touch |
| 5 | Command list (React Aria `Autocomplete` + `Menu`) with the on-screen keyboard: the page scrolls the field above the keyboard, typing filters, one tap on a result fires it | Pass |

## What changes

1. **Phone sidebar opens with a button, never an edge swipe** (spec §6.7). The workspace name in the header opens the
   sheet; a swipe closes it. No `SwipeArea` on the left edge.
2. **Drag data is `text/plain`.** Board cards, backlog rows, tabs and sidebar items carry their id as `text/plain`
   (with a prefix such as `pdt:ticket:PDT-44` so stray text drops are ignored), not a custom MIME type.
3. **Draggable rows need a drag handle.** React Aria warns that draggable `GridList` items must contain a
   `<Button slot="drag">` so keyboard and screen-reader users can drag; `ListRow` and board cards get one (visible on
   hover and focus, always on touch).
4. **Style: Nova** (`-p nova`). Its button sizes already include 24px (`xs`) and 28px (`sm`), Graphite's two control
   heights; we map our `sm`/`md` onto them and keep 6px corners through `--radius`.
5. **Dependencies we drop after init:** `next-themes` (our theme code drives Sonner's `theme` prop) and the Geist
   font package.

## Found while migrating (Android, real touch)

- A long press on a **natively draggable** element (`draggable` attribute) starts Android's drag instead of the
  context menu. Tabs are now draggable only when the pointer isn't coarse; on touch they move from the long-press
  menu (workspaces spec §4.6). React Aria `List` rows aren't affected: they drag from a handle.

## Still open

- **iOS Safari on a device is still unverified** (PDT-122 below checked it in Playwright's WebKit only). Check from a
  phone over Tailscale when one is available, or install Xcode for the iOS Simulator. Every drag already has a menu
  equivalent (spec §3), so a failure there degrades, not blocks.
- Keyboard drag (Enter to pick up, Tab to a target, Enter to drop) was not exercised here; it's covered by the
  component tests once the drag handle exists.
- The spike's own markup showed "No results" next to matches (the empty state was outside the list); the real
  `CommandList` puts it inside, as shadcn's examples do.

## Phone redesign check (PDT-122, 2026-10-23)

PDT-7's phone UI walked in Playwright: WebKit with the iPhone 13 profile (and `navigator.platform` set to `iPhone`, so
React Aria takes its iOS paths) and Chromium with the Pixel 7 profile, touch input through CDP
(`Input.dispatchTouchEvent`) on Chromium and synthetic touch `PointerEvent`s on WebKit. Still no Xcode or iPhone here.

| Check | iOS (WebKit) | Android (Chromium) |
|---|---|---|
| Every drawer screen (PM, Sessions, Inbox, Tickets, Board, Workflows, Personas, Files, Settings and its sections) renders with no page errors and no sideways overflow | Pass | Pass |
| Drawer: opens from ☰; Back closes it (URL unchanged); a tap outside closes it | Pass | Pass |
| Drawer: a swipe left closes it | Not checkable (no touch input in WebKit) | Pass |
| List screen → detail (`/tickets` → `/tickets/SA-8`) and Back returns | Pass | Pass |
| Text fields, the composer and the terminal's input are ≥ 16px (iOS zooms the page on focus below that) | Pass | Pass |
| Long press on a list row (tickets, sessions) opens its menu and does nothing else | **Failed, fixed** (below) | Pass |
| Long press on a tab or a board card opens its menu (React Aria's own timer on iOS) | Pass | Not checkable (CDP touch fires no `contextmenu`; the spike passed it on a device) |
| Board: a swipe changes the column | Not checkable | Pass |

**Fixed:** iOS sends no `click` after a long press, and without one React Aria fires the row's press itself when the
finger lifts, so a long press on a `List`/`DataList` row opened its menu and then opened the row (a ticket long press
navigated to the ticket). `useRowMenu` now ends React Aria's press (a `pointercancel`) when its timer opens the menu.

**Needs a device** (steps for whoever has one):
1. Composer above the keyboard: open `/pm`, tap the box; the composer stays above the keyboard and the top bar stays
   visible; type, rotate, dismiss the keyboard; the layout returns to full height.
2. Terminal: open a running shell tab, tap the terminal; the keyboard opens without zooming the page; the key bar sits
   above it; a long press selects a word with no iOS callout or magnifier, Copy then Paste (⋯) works.
3. Safe areas: `index.html` has no `viewport-fit=cover`, so `env(safe-area-inset-*)` is 0 and Safari keeps the page out
   of the notch and home-indicator areas itself. Check the bottom bars (ticket actions, Inbox decisions, composer) in
   portrait and landscape aren't under Safari's toolbar.
4. Swipe-back from the left edge on a detail screen goes up one screen, like ←; on the drawer it closes the drawer.
5. A long press on a ticket row, a tab and a board card opens the menu only: no text selection, callout, or opening
   the item.
