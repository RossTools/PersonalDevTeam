import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { Run } from '../api/gen/Run';
import type { Session } from '../api/gen/Session';
import { stubApi } from '../test-utils';
import { sessionInfo, type ItemInfo } from '../workspaces/useWorkspaces';
import { attentionOf, badgedSvg, useFaviconBadge, type AttentionLevel, type AttentionSources } from './attention';

const session = (id: string, more: Partial<Session> = {}): Session => ({
  id, projectId: 'p1', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: id,
  cwd: '/x', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'working', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: null, ...more,
});

const item = (id: string, kind: InboxItem['kind'], more: Partial<InboxItem> = {}): InboxItem => ({
  id, kind, group: kind === 'notice' ? 'notice' : kind === 'session' ? 'session' : 'question', status: 'open', projectId: 'p1',
  ticketId: null, ticketKey: null, ticketTitle: null, attemptId: null, sessionId: null, runId: null, stepId: null,
  personaId: null, personaName: null, changeRequest: null, title: id, body: {}, choices: [], resolution: null, createdAt: 1, resolvedAt: null, ...more,
});

const pm = (s: Session, open = true): PmSessionInfo => ({
  session: s, open, title: s.title, unseen: s.unseen, lastActivity: 1, rotation: 'none', reason: null, journal: null, via: null, resetFrom: null,
});

const runInfo = (id: string, unseen: boolean): ItemInfo => ({
  item: { kind: 'run', id }, title: id, status: 'failed', activity: null, unseen, projectId: 'p1', worktreeId: null,
  run: { state: 'failed' } as Run,
});

const none: AttentionSources = { inbox: [], items: [], pm: [], pmShown: null };

describe('attentionOf', () => {
  it('is none when nothing waits', () => {
    expect(attentionOf(none)).toEqual({ level: 'none', count: 0 });
    // A working session, a seen idle one, a finished one that once needed the user and a seen run are quiet.
    const items = [session('a'), session('b', { activity: 'idle' }), session('c', { status: 'finished', activity: 'needs_you' })]
      .map(sessionInfo).concat(runInfo('r', false));
    expect(attentionOf({ ...none, items })).toEqual({ level: 'none', count: 0 });
  });

  it('counts the Inbox\'s open items and nothing else', () => {
    const inbox = [item('q', 'question'), item('n', 'notice'), item('old', 'question', { status: 'resolved' })];
    const items = [sessionInfo(session('s', { activity: 'needs_you' })), sessionInfo(session('d', { activity: 'idle', unseen: true }))];
    const reply = pm(session('pm1', { activity: 'idle', unseen: true }));
    expect(attentionOf({ inbox, items, pm: [reply], pmShown: null }).count).toBe(2);
    expect(attentionOf({ ...none, items, pm: [reply] }).count).toBe(0);
  });

  it('is needs_you when a session needs the user, from its own activity or its Inbox item', () => {
    const items = [sessionInfo(session('s', { activity: 'needs_you' }))];
    expect(attentionOf({ ...none, items })).toEqual({ level: 'needs_you', count: 0 });
    const waiting = item('session:s', 'session', { sessionId: 's' });
    expect(attentionOf({ ...none, inbox: [waiting] })).toEqual({ level: 'needs_you', count: 1 });
    expect(attentionOf({ ...none, inbox: [waiting], items })).toEqual({ level: 'needs_you', count: 1 });
  });

  it('is inbox for any other open Inbox item', () => {
    for (const kind of ['question', 'approval', 'secret', 'gate', 'stalled', 'notice'] as const) {
      expect(attentionOf({ ...none, inbox: [item('i', kind)] })).toEqual({ level: 'inbox', count: 1 });
    }
    expect(attentionOf({ ...none, inbox: [item('i', 'question', { status: 'resolved' })] }).level).toBe('none');
  });

  it('is idle for an unseen finished session or run and an unread PM reply', () => {
    const done = sessionInfo(session('d', { activity: 'idle', unseen: true }));
    expect(attentionOf({ ...none, items: [done] })).toEqual({ level: 'idle', count: 0 });
    expect(attentionOf({ ...none, items: [runInfo('r', true)] }).level).toBe('idle');
    const reply = pm(session('pm1', { activity: 'idle', unseen: true, projectId: 'host' }));
    expect(attentionOf({ ...none, pm: [reply] })).toEqual({ level: 'idle', count: 0 });
  });

  it('leaves workflow step sessions out of idle, but not out of needs_you (PDT-262)', () => {
    const persona = { id: 'coder', name: 'Coder', version: 1, stepId: 'st1' };
    const step = sessionInfo(session('c', { activity: 'idle', unseen: true, persona }));
    expect(attentionOf({ ...none, items: [step] })).toEqual({ level: 'none', count: 0 });
    const ended = sessionInfo(session('q', { status: 'finished', activity: null, unseen: true, persona }));
    expect(attentionOf({ ...none, items: [step, ended] }).level).toBe('none');
    const mine = sessionInfo(session('m', { activity: 'idle', unseen: true }));
    expect(attentionOf({ ...none, items: [step, mine] }).level).toBe('idle');
    const asking = sessionInfo(session('r', { activity: 'needs_you', unseen: true, persona }));
    expect(attentionOf({ ...none, items: [asking] }).level).toBe('needs_you');
  });

  it('skips the PM conversation on screen, a running PM turn and an ended conversation', () => {
    const s = session('pm1', { activity: 'idle', unseen: true });
    expect(attentionOf({ ...none, pm: [pm(s)], pmShown: 'pm1' }).level).toBe('none');
    expect(attentionOf({ ...none, pm: [pm({ ...s, activity: 'working' })] }).level).toBe('none');
    expect(attentionOf({ ...none, pm: [pm(s, false)] }).level).toBe('none');
  });

  it('takes the most important level: needs_you over inbox over idle', () => {
    const done = sessionInfo(session('d', { activity: 'idle', unseen: true }));
    const needs = sessionInfo(session('s', { activity: 'needs_you' }));
    const q = item('q', 'question');
    expect(attentionOf({ ...none, inbox: [q], items: [done] }).level).toBe('inbox');
    expect(attentionOf({ ...none, inbox: [q], items: [done, needs] }).level).toBe('needs_you');
    expect(attentionOf({ ...none, items: [done, needs] }).level).toBe('needs_you');
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
    document.documentElement.style.setProperty('--pdt-failed', 'red');
    document.documentElement.style.setProperty('--pdt-waiting', 'amber');
    document.documentElement.style.setProperty('--pdt-accent', 'blue');
    document.documentElement.style.setProperty('--pdt-surface', 'paper');
  });
  afterEach(() => {
    link.remove();
    document.documentElement.removeAttribute('style');
    vi.unstubAllGlobals();
  });

  it('draws a red dot for a session that needs the user, amber for the Inbox and the accent for news, ringed in the surface colour', () => {
    const needs = badgedSvg(plain, 'needs_you');
    expect(needs).toMatch(/<rect width="32" height="32"\/><circle[^>]*fill="paper"\/><circle[^>]*fill="red"\/><\/svg>$/);
    expect(badgedSvg(plain, 'inbox')).toContain('fill="amber"');
    expect(badgedSvg(plain, 'idle')).toContain('fill="blue"');
  });

  it('places and sizes the dot by the icon\'s viewBox', () => {
    const big = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64"/></svg>';
    expect(badgedSvg(big, 'inbox')).toContain('<circle cx="50" cy="14" r="16" fill="paper"/><circle cx="50" cy="14" r="12" fill="amber"/>');
    expect(badgedSvg(plain, 'inbox')).toContain('<circle cx="25" cy="7" r="8" fill="paper"/><circle cx="25" cy="7" r="6" fill="amber"/>');
  });

  it('swaps the icon while something wants the user and restores the plain one', async () => {
    const calls = stubApi({ 'GET /favicon.svg': () => new Response(plain) });
    const { rerender, unmount } = renderHook(({ level }) => useFaviconBadge(level), {
      initialProps: { level: 'none' as AttentionLevel },
    });
    expect(link.getAttribute('href')).toBe('/favicon.svg');
    rerender({ level: 'needs_you' });
    await waitFor(() => expect(decodeURIComponent(link.getAttribute('href')!)).toContain('fill="red"'));
    rerender({ level: 'inbox' });
    await waitFor(() => expect(decodeURIComponent(link.getAttribute('href')!)).toContain('fill="amber"'));
    rerender({ level: 'idle' });
    await waitFor(() => expect(decodeURIComponent(link.getAttribute('href')!)).toContain('fill="blue"'));
    rerender({ level: 'none' });
    expect(link.getAttribute('href')).toBe('/favicon.svg');
    rerender({ level: 'inbox' });
    await waitFor(() => expect(link.getAttribute('href')).toMatch(/^data:image\/svg\+xml,/));
    unmount();
    expect(link.getAttribute('href')).toBe('/favicon.svg');
    // The plain icon is fetched once.
    expect(calls.filter((c) => c.path === '/favicon.svg')).toHaveLength(1);
  });
});
