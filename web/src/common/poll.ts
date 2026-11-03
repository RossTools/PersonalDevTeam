// How often queries refetch: one set of tiers for the whole UI, so a screen picks how fresh it needs to be rather than
// a number of milliseconds. The UI's own writes invalidate what they change at once; polls catch what changes without
// it (agents, runs, workflows, other tabs).

export const POLL = {
  /** What's on screen and moves while you watch: sessions, runs, tickets, the Inbox, workspaces. */
  live: 2000,
  /** What changes now and then: a workflow, changes under review, a tab's overview, a connection's status. */
  steady: 5000,
  /** What changes rarely without this UI: worktrees, a ticket list's counts, a connection's tools. */
  slow: 10_000,
  /** A long list scrolled back (each loaded page refetches), a document's version. */
  paged: 15_000,
  /** What a stream pushes while it is connected, as a safety net. */
  pushed: 30_000,
  /** What hardly changes: the update check, ticket labels. */
  idle: 60_000,
} as const;

/**
 * A paged query's poll: TanStack refetches every loaded page, so past two pages (a long scroll back) it polls at
 * `POLL.paged`, so that a 1,000-row list isn't ten requests every 2 s.
 */
export const pagedPoll = (query: { state: { data?: { pages: unknown[] } } }) => ((query.state.data?.pages.length ?? 0) > 2 ? POLL.paged : POLL.live);

let streamOpen = false;

/** The notification stream says whether it is connected (it invalidates what it pushes when that changes). */
export function setStreamOpen(open: boolean) {
  streamOpen = open;
}

/**
 * The poll for what the notification stream pushes (the Updates): slow while the stream is connected, live while it
 * isn't. Read after each fetch, so the stream invalidates its queries when it connects or drops to switch at once.
 */
export const streamPoll = () => (streamOpen ? POLL.pushed : POLL.live);
