import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { Session } from '../api/gen/Session';
import { stubApi } from '../test-utils';
import { sessionInfo } from '../workspaces/useWorkspaces';
import { attentionOf, badgedSvg, useFaviconBadge, type AttentionSources } from './attention';

const session = (id: string, more: Partial<Session> = {}): Session => ({
  id, projectId: 'p1', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: id,
  cwd: '/x', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'working', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: null, ...more,
});

const item = (id: string, kind: InboxItem['kind'], more: Partial<InboxItem> = {}): InboxItem => ({
  id, kind, group: kind === 'notice' ? 'notice' : kind === 'session' ? 'session' : 'question', status: 'open', projectId: 'p1',
  ticketId: null, ticketKey: null, ticketTitle: null, attemptId: null, sessionId: null, runId: null, stepId: null,
  personaId: null, personaName: null, title: id, body: {}, choices: [], resolution: null, createdAt: 1, resolvedAt: null, ...more,
});

const pm = (s: Session, open = true): PmSessionInfo => ({
  session: s, open, title: s.title, unseen: s.unseen, lastActivity: 1, rotation: 'none', reason: null, journal: null,
});

const none: AttentionSources = { inbox: [], unreadUpdates: 0, items: [], pm: [], pmShown: null };

describe('attentionOf', () => {
  it('is none when nothing wants the user', () => {
    expect(attentionOf(none)).toEqual({ level: 'none', count: 0, blocking: 0 });
    // A working session, a seen idle one and a finished one that once needed the user are quiet.
    const items = [session('a'), session('b', { activity: 'idle' }), session('c', { status: 'finished', activity: 'needs_you' })].map(sessionInfo);
    expect(attentionOf({ ...none, items }).level).toBe('none');
  });

  it('blocks on questions, approvals, secret requests and sessions that need the user', () => {
    for (const kind of ['question', 'approval', 'secret', 'gate', 'stalled', 'session'] as const) {
      expect(attentionOf({ ...none, inbox: [item('i', kind)] })).toEqual({ level: 'blocking', count: 1, blocking: 1 });
    }
    const items = [sessionInfo(session('s', { activity: 'needs_you' }))];
    expect(attentionOf({ ...none, items })).toEqual({ level: 'blocking', count: 1, blocking: 1 });
  });

  it('is only news for unheard notices, unread Updates, unseen sessions and unread PM replies', () => {
    expect(attentionOf({ ...none, inbox: [item('n', 'notice')] }).level).toBe('unseen');
    expect(attentionOf({ ...none, unreadUpdates: 2 })).toEqual({ level: 'unseen', count: 2, blocking: 0 });
    const done = sessionInfo(session('d', { activity: 'idle', unseen: true }));
    expect(attentionOf({ ...none, items: [done] })).toEqual({ level: 'unseen', count: 1, blocking: 0 });
    const reply = pm(session('pm1', { activity: 'idle', unseen: true, projectId: 'host' }));
    expect(attentionOf({ ...none, pm: [reply] })).toEqual({ level: 'unseen', count: 1, blocking: 0 });
  });

  it('skips the PM conversation on screen, a running PM turn and an ended conversation', () => {
    const s = session('pm1', { activity: 'idle', unseen: true });
    expect(attentionOf({ ...none, pm: [pm(s)], pmShown: 'pm1' }).level).toBe('none');
    expect(attentionOf({ ...none, pm: [pm({ ...s, activity: 'working' })] }).level).toBe('none');
    expect(attentionOf({ ...none, pm: [pm(s, false)] }).level).toBe('none');
  });

  it('counts a session once, at its most urgent, and adds everything else', () => {
    const waiting = session('s', { activity: 'needs_you', unseen: true });
    const sources: AttentionSources = {
      inbox: [item('session:s', 'session', { sessionId: 's' }), item('q', 'question'), item('n', 'notice'), item('old', 'question', { status: 'resolved' })],
      unreadUpdates: 1,
      items: [sessionInfo(waiting), sessionInfo(session('d', { activity: 'idle', unseen: true }))],
      pm: [pm(session('pm1', { activity: 'idle', unseen: true }))],
      pmShown: null,
    };
    // Blocking: s, q. News: n, d, pm1, one Update.
    expect(attentionOf(sources)).toEqual({ level: 'blocking', count: 6, blocking: 2 });
  });
});

describe('favicon badge', () => {
  const plain = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32"/></svg>\n';
  let link: HTMLLinkElement;

  beforeEach(() => {
    link = document.createElement('link');
    link.rel = 'icon';
    link.setAttribute('href', '/favicon.svg');
    document.head.appendChild(link);
    document.documentElement.style.setProperty('--pdt-waiting', 'amber');
    document.documentElement.style.setProperty('--pdt-accent', 'blue');
    document.documentElement.style.setProperty('--pdt-surface', 'paper');
  });
  afterEach(() => {
    link.remove();
    document.documentElement.removeAttribute('style');
    vi.unstubAllGlobals();
  });

  it('draws an amber dot for a wait and an accent dot for news, ringed in the surface colour', () => {
    const blocking = badgedSvg(plain, 'blocking');
    expect(blocking).toMatch(/<rect width="32" height="32"\/><circle[^>]*fill="paper"\/><circle[^>]*fill="amber"\/><\/svg>$/);
    expect(badgedSvg(plain, 'unseen')).toContain('fill="blue"');
  });

  it('swaps the icon while something wants the user and restores the plain one', async () => {
    const calls = stubApi({ 'GET /favicon.svg': () => new Response(plain) });
    const { rerender, unmount } = renderHook(({ level }) => useFaviconBadge(level), {
      initialProps: { level: 'none' as 'none' | 'unseen' | 'blocking' },
    });
    expect(link.getAttribute('href')).toBe('/favicon.svg');
    rerender({ level: 'blocking' });
    await waitFor(() => expect(decodeURIComponent(link.getAttribute('href')!)).toContain('fill="amber"'));
    rerender({ level: 'unseen' });
    await waitFor(() => expect(decodeURIComponent(link.getAttribute('href')!)).toContain('fill="blue"'));
    rerender({ level: 'none' });
    expect(link.getAttribute('href')).toBe('/favicon.svg');
    rerender({ level: 'blocking' });
    await waitFor(() => expect(link.getAttribute('href')).toMatch(/^data:image\/svg\+xml,/));
    unmount();
    expect(link.getAttribute('href')).toBe('/favicon.svg');
    // The plain icon is fetched once.
    expect(calls.filter((c) => c.path === '/favicon.svg')).toHaveLength(1);
  });
});
