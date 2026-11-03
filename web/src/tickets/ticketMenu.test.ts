import { describe, expect, it, vi } from 'vitest';
import { QueryClient } from '@tanstack/react-query';
import type { MenuEntry } from '@ui';
import { ticketMenu } from './actions';

const base = { id: 't1', projectId: 'p1', key: 'PDT-1', title: 'A ticket', stage: 'in_progress' as const, priority: 'medium' as const };
const confirm = vi.fn(async () => ({ ok: true }) as never);
const names = (entries: MenuEntry[]) =>
  entries.map((e) => ('label' in e ? e.label : 'submenu' in e ? `${e.submenu} ›` : '—'));

describe('ticketMenu', () => {
  it('has the entries every ticket gets', () => {
    expect(names(ticketMenu({ qc: new QueryClient(), confirm, ticket: base }))).toEqual(['Move to ›', 'Priority ›', '—', 'Delete…']);
  });

  it('adds each option where the ticket allows it, in one order', () => {
    const ticket = {
      ...base, waiting: true, pickupFailed: 'boom',
      attempt: { id: 'a1', state: 'active' as const, branch: 'b' },
      parts: { done: 0, total: 2, stages: [] } as never,
    };
    const entries = ticketMenu({
      qc: new QueryClient(), confirm, ticket, watch: { watches: [] }, onOpen: () => {}, onStart: () => {}, startPart: () => {},
      onMakeParts: () => {}, onMerge: () => {}, onFileIssue: () => {}, abandon: true, rank: true,
    });
    expect(names(entries)).toEqual([
      'Watch', 'Open', 'Start…', 'Stop waiting', 'Try auto-start again', 'Move to ›', 'Priority ›', 'Start next part',
      'Make into parts…', 'Merge…', 'Create GitHub issue…', 'Abandon attempt…', 'Move to top', 'Move to bottom', '—', 'Delete…',
    ]);
  });

  it('leaves out Merge, Abandon and Start next part when there is nothing for them', () => {
    const ticket = { ...base, stage: 'done' as const, attempt: { id: 'a1', state: 'merged' as const, branch: 'b' }, parts: { done: 2, total: 2 } as never };
    const entries = ticketMenu({ qc: new QueryClient(), confirm, ticket, startPart: () => {}, onMerge: () => {}, abandon: true });
    expect(names(entries)).toEqual(['Move to ›', 'Priority ›', '—', 'Delete…']);
  });

  it('offers Refine on a Draft or Ready ticket', () => {
    expect(names(ticketMenu({ qc: new QueryClient(), confirm, ticket: { ...base, stage: 'ready' } }))[0]).toBe('Refine');
  });
});
