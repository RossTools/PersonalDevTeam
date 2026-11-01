import { act, screen, within } from '@testing-library/react';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import type { TicketKeyProps } from '../common/TicketKey';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { Board } from './Board';
import { page, params, project, summary } from './fixtures';

// Each card shows its key once: counting the key chip's renders counts the card's (PDT-136).
const renders = vi.hoisted(() => new Map<string, number>());
vi.mock('../common/TicketKey', async (importOriginal) => {
  const m = await importOriginal<typeof import('../common/TicketKey')>();
  return {
    ...m,
    TicketKey: (p: TicketKeyProps) => {
      renders.set(p.ticketKey, (renders.get(p.ticketKey) ?? 0) + 1);
      return <m.TicketKey {...p} />;
    },
  };
});

beforeEach(() => renders.clear());
afterEach(() => vi.unstubAllGlobals());

const attempt = { id: 'a1', branch: 'pdt/ALP-2-cache', baseBranch: 'main', worktreeId: 'w1', state: 'active' as const, createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, ci: null, pr: null };
const agent: Session = {
  id: 's1', projectId: project.id, worktreeId: 'w1', kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Claude · ALP-2',
  cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'working', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: 'a1', ticket: { id: 't2', key: 'ALP-2' },
  mode: 'terminal', model: null, permissionMode: null, persona: null, background: [],
};

let qc: QueryClient;
function Grab() {
  qc = useQueryClient();
  return null;
}

/** Every poll answers anew (new JSON each time), like the server. */
function stub(state: { title1: string; activity: Session['activity'] }) {
  stubApi({
    'GET /api/tickets': (c: Call) => {
      const stage = params(c.path).stage;
      const items = stage === 'todo' ? [summary(1, { stage: 'todo', title: state.title1 }), summary(4, { stage: 'todo' })]
        : stage === 'in_progress' ? [summary(2, { stage: 'in_progress', title: 'Cache', attempt })]
        : [];
      return page(items);
    },
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/sessions`]: () => [{ ...agent, activity: state.activity }],
  });
}

/** A poll of every query; TanStack tells observers on a timer, so wait for that too. */
const poll = () => act(async () => {
  await qc.refetchQueries();
  await new Promise((r) => setTimeout(r, 10));
});

describe('board renders', () => {
  it('re-renders no card on a poll that changes nothing, and only the changed card otherwise', async () => {
    const state = { title1: 'Ticket 1', activity: 'working' as Session['activity'] };
    stub(state);
    renderWithQuery(<><Grab /><Board projectId={project.id} onPeek={() => {}} phone={false} /></>);
    expect(await within(screen.getByRole('region', { name: 'In progress' })).findByText('Cache')).toBeInTheDocument();
    await poll();
    const before = Object.fromEntries(renders);
    expect(before['ALP-1']).toBeGreaterThan(0);

    await poll();
    await poll();
    expect(Object.fromEntries(renders)).toEqual(before);

    // A ticket's title changes: its card re-renders, the others don't.
    state.title1 = 'Renamed';
    await poll();
    expect(await screen.findByText('Renamed')).toBeInTheDocument();
    expect(renders.get('ALP-1')).toBeGreaterThan(before['ALP-1']);
    expect(renders.get('ALP-2')).toBe(before['ALP-2']);
    expect(renders.get('ALP-4')).toBe(before['ALP-4']);

    // A session's state changes: only its ticket's card (the dot) re-renders.
    const after = Object.fromEntries(renders);
    state.activity = 'needs_you';
    await poll();
    expect(renders.get('ALP-2')).toBeGreaterThan(after['ALP-2']);
    expect(renders.get('ALP-1')).toBe(after['ALP-1']);
    expect(renders.get('ALP-4')).toBe(after['ALP-4']);
  });
});
