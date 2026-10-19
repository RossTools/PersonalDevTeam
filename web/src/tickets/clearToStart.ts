import { useEffect } from 'react';
import { toast } from '@ui';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { useStartTicket } from './actions';

const KEY = 'pdt.tickets.clearToastShown';

/** The tickets already toasted in this browser session (this page only when storage is blocked). */
const memory = new Set<string>();

function shown(): Set<string> {
  try {
    const v = JSON.parse(sessionStorage.getItem(KEY) ?? '[]');
    if (Array.isArray(v)) for (const id of v) if (typeof id === 'string') memory.add(id);
  } catch {
    // Unreadable or blocked: the page's own memory.
  }
  return memory;
}

function remember(ids: Set<string>) {
  try {
    sessionStorage.setItem(KEY, JSON.stringify([...ids]));
  } catch {
    // Blocked: remembered for this page only.
  }
}

/** Forgets what was toasted (tests). */
export function resetClearToasts() {
  memory.clear();
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    // Nothing stored.
  }
}

type ClearTicket = Pick<TicketSummary, 'id' | 'projectId' | 'key' | 'clearToStart'>;

/**
 * Toasts "<KEY> is clear to start" (with Start) the first time a waiting ticket on screen is clear, once per ticket
 * per browser session. Idempotent: safe on every poll.
 */
export function useClearToStartToasts(tickets: ClearTicket[]) {
  const start = useStartTicket();
  const clear = tickets.filter((t) => t.clearToStart);
  const ids = clear.map((t) => t.id).join();
  useEffect(() => {
    if (clear.length === 0) return;
    const seen = shown();
    const fresh = clear.filter((t) => !seen.has(t.id));
    if (fresh.length === 0) return;
    for (const t of fresh) {
      seen.add(t.id);
      toast.info(`${t.key} is clear to start`, { action: { label: 'Start', onClick: () => start(t) } });
    }
    remember(seen);
    // Only a change in which tickets are clear matters; `clear` is new every render.
  }, [ids]);
}
