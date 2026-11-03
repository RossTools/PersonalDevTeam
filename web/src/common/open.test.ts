import { afterEach, describe, expect, it, vi } from 'vitest';
import { openExternal } from './open';
import { inboxTargets } from '../inbox/model';
import type { InboxItem } from '../api/gen/InboxItem';

afterEach(() => vi.restoreAllMocks());

describe('openExternal', () => {
  it('opens a new tab that cannot reach back into PDT', () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    openExternal('https://example.com/x');
    expect(open).toHaveBeenCalledWith('https://example.com/x', '_blank', 'noopener,noreferrer');
  });
});

describe('inboxTargets', () => {
  const item = (o: Partial<InboxItem>) => ({ projectId: 'p1', ticketId: null, sessionId: null, ...o }) as InboxItem;
  it('gives the ticket and session an item opens, in its project', () => {
    expect(inboxTargets(item({ ticketId: 't1', sessionId: 's1' }))).toEqual({
      ticket: { id: 't1', projectId: 'p1' },
      session: { id: 's1', projectId: 'p1' },
    });
  });
  it('has neither without a project', () => {
    expect(inboxTargets(item({ projectId: null, ticketId: 't1', sessionId: 's1' }))).toEqual({ ticket: null, session: null });
  });
});
