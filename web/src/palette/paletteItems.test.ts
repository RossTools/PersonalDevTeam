import { describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Run } from '../api/gen/Run';
import type { Session } from '../api/gen/Session';
import type { Tab } from '../api/gen/Tab';
import type { TicketSummary } from '../api/gen/TicketSummary';
import type { Workspace } from '../api/gen/Workspace';
import { runInfo, sessionInfo, ticketInfo } from '../workspaces/useWorkspaces';
import { ACTIONS, GO_TO, paletteItems, type PaletteContext, type PaletteHandlers, type PaletteItem } from './paletteItems';

const project = (id: string, name: string, key: string): Project => ({
  id, name, key, repoPath: `/r/${id}`, baseBranch: 'main', createdAt: 0, githubRepo: null,
});

const session = (over: Partial<Session> & { id: string }): Session => ({
  projectId: 'p1', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: over.id,
  cwd: '/r/p1', status: 'running', exitCode: null, createdAt: 0, endedAt: null, externalSessionId: null,
  freshStartedAt: null, activity: 'idle', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, ...over,
});

const run: Run = {
  id: 'r1', projectId: 'p1', worktreeId: 'wt1', branch: 'feature-x', name: 'pdt', kind: 'run', state: 'ready', port: 8500,
  url: null, dataDir: null, cwd: '/r/p1/.worktrees/feature-x', exitCode: null, error: null, createdAt: 0, updatedAt: 0,
};

const tab = (id: string, workspaceId: string, position: number, item: { sessionId?: string; runId?: string }): Tab => ({
  id, workspaceId, position, sessionId: item.sessionId ?? null, runId: item.runId ?? null, ticketId: null,
});

const workspaces: Workspace[] = [
  {
    id: 'w1', name: 'PersonalDevTeam', projectId: 'p1', position: 0, createdAt: 0,
    tabs: [tab('t1', 'w1', 0, { sessionId: 's1' }), tab('t2', 'w1', 1, { runId: 'r1' })],
  },
  { id: 'w2', name: 'Scratch', projectId: null, position: 1, createdAt: 0, tabs: [tab('t3', 'w2', 0, { sessionId: 's2' })] },
];

const sessions = [
  session({ id: 's1', title: 'fix login', worktreeId: 'wt1', cwd: '/r/p1/.worktrees/fix-login' }),
  session({ id: 's2', title: 'tickets schema', activity: 'needs_you' }),
  session({ id: 's3', title: 'no tab agent', projectId: 'p2', cwd: '/r/p2' }),
  session({ id: 's4', title: 'loose shell', kind: 'shell', agent: null }),
];

function handlers(): PaletteHandlers {
  return {
    selectWorkspace: vi.fn(), selectTab: vi.fn(), focusItem: vi.fn(), projectSettings: vi.fn(), hostSettings: vi.fn(),
    newSession: vi.fn(), openChooser: vi.fn(), renameTab: vi.fn(), closeTab: vi.fn(), moveTab: vi.fn(),
    moveTabToNewWorkspace: vi.fn(), setTheme: vi.fn(), showKeys: vi.fn(), showFiles: vi.fn(), newTicket: vi.fn(), openTicket: vi.fn(), startTicket: vi.fn(),
  };
}

function ctx(over: Partial<PaletteContext> = {}): PaletteContext {
  return {
    workspaces,
    infos: [...sessions.map((s) => sessionInfo(s, 0)), runInfo(run, 0)],
    projects: [project('p1', 'PersonalDevTeam', 'PDT'), project('p2', 'Demo', 'DEMO')],
    tab: workspaces[0].tabs[0],
    place: { projectId: 'p1', worktreeId: 'wt1' },
    theme: 'system',
    handlers: handlers(),
    ...over,
  };
}

const byId = (items: PaletteItem[], id: string) => {
  const it = items.find((i) => i.id === id);
  if (!it) throw new Error(`no item ${id} in ${items.map((i) => i.id).join(', ')}`);
  return it;
};
const ids = (items: PaletteItem[]) => items.map((i) => i.id);

describe('paletteItems', () => {
  it('lists Go to first, then Actions', () => {
    const items = paletteItems(ctx());
    const groups = items.map((i) => i.group);
    expect(groups.lastIndexOf(GO_TO)).toBeLessThan(groups.indexOf(ACTIONS));
  });

  it('goes to every workspace and every tab, tabs with their agent, workspace, state, project key and branch', () => {
    const c = ctx();
    const items = paletteItems(c);
    expect(byId(items, 'workspace:w2').label).toBe('Workspace · Scratch');
    const t1 = byId(items, 'tab:t1');
    expect(t1).toMatchObject({ label: 'fix login', detail: 'Claude · PersonalDevTeam', group: GO_TO });
    expect(t1.keywords).toEqual(expect.arrayContaining(['PDT', 'fix-login', 'Claude']));
    expect(t1.state?.label).toBe('Idle');
    expect(byId(items, 'tab:t2')).toMatchObject({ label: 'pdt · feature-x', detail: 'PersonalDevTeam', keywords: expect.arrayContaining(['feature-x']) });
    expect(byId(items, 'tab:t3').state?.label).toBe('Needs you');
    byId(items, 'tab:t3').run!();
    expect(c.handlers.selectTab).toHaveBeenCalledWith(workspaces[1].tabs[0]);
    byId(items, 'workspace:w1').run!();
    expect(c.handlers.selectWorkspace).toHaveBeenCalledWith('w1');
  });

  it('goes to agents without a tab (not shells), opening them in their project', () => {
    const c = ctx();
    const items = paletteItems(c);
    expect(ids(items).filter((i) => i.startsWith('agent:'))).toEqual(['agent:s3']);
    const agent = byId(items, 'agent:s3');
    expect(agent.keywords).toContain('DEMO');
    agent.run!();
    expect(c.handlers.focusItem).toHaveBeenCalledWith({ kind: 'session', id: 's3' }, 'p2');
  });

  it("goes to each project's settings and this host's", () => {
    const c = ctx();
    const items = paletteItems(c);
    expect(byId(items, 'settings:p2')).toMatchObject({ label: 'Settings · Demo', keywords: ['DEMO'] });
    byId(items, 'settings:p2').run!();
    expect(c.handlers.projectSettings).toHaveBeenCalledWith('p2');
    byId(items, 'settings:host').run!();
    expect(c.handlers.hostSettings).toHaveBeenCalled();
  });

  it('starts sessions at the current place, only when there is one', () => {
    const c = ctx();
    const items = paletteItems(c);
    expect(ids(items)).toEqual(expect.arrayContaining(['new:claude', 'new:codex', 'new:shell']));
    byId(items, 'new:codex').run!();
    expect(c.handlers.newSession).toHaveBeenCalledWith('codex', { projectId: 'p1', worktreeId: 'wt1' });
    expect(ids(paletteItems(ctx({ place: null }))).some((i) => i.startsWith('new:'))).toBe(false);
  });

  it('shows the Ctrl+] key for actions that have one', () => {
    const items = paletteItems(ctx());
    expect(byId(items, 'run').shortcut).toBe('Ctrl+] c');
    expect(byId(items, 'tab:rename').shortcut).toBe('Ctrl+] r');
    expect(byId(items, 'tab:close').shortcut).toBe('Ctrl+] x');
    expect(byId(items, 'needs-you').shortcut).toBe('Ctrl+] a');
    expect(byId(items, 'keys').shortcut).toBe('Ctrl+] ?');
    expect(byId(items, 'new:claude').shortcut).toBeUndefined();
  });

  it('acts on the current tab; without one there are no tab actions; runs cannot be renamed', () => {
    const c = ctx();
    const items = paletteItems(c);
    byId(items, 'tab:close').run!();
    expect(c.handlers.closeTab).toHaveBeenCalledWith(workspaces[0].tabs[0]);
    byId(items, 'tab:rename').run!();
    expect(c.handlers.renameTab).toHaveBeenCalledWith(workspaces[0].tabs[0]);
    expect(ids(paletteItems(ctx({ tab: null }))).some((i) => i.startsWith('tab:') && !/^tab:t\d/.test(i))).toBe(false);
    const onRun = ids(paletteItems(ctx({ tab: workspaces[0].tabs[1] })));
    expect(onRun).toContain('tab:close');
    expect(onRun).not.toContain('tab:rename');
  });

  it('moves the tab in a second step: the other workspaces, then a new one by name', () => {
    const c = ctx();
    const move = byId(paletteItems(c), 'tab:move');
    expect(move.run).toBeUndefined();
    if (move.next?.kind !== 'list') throw new Error('expected a list step');
    expect(move.next.items.map((i) => i.label)).toEqual(['Scratch', 'New workspace…']);
    byId(move.next.items, 'move:w2').run!();
    expect(c.handlers.moveTab).toHaveBeenCalledWith(workspaces[0].tabs[0], 'w2');
    const named = byId(move.next.items, 'move:new').next;
    if (named?.kind !== 'input') throw new Error('expected an input step');
    named.run('Release');
    expect(c.handlers.moveTabToNewWorkspace).toHaveBeenCalledWith(workspaces[0].tabs[0], 'Release');
  });

  it('offers Next needs you only when something needs you', () => {
    const c = ctx();
    byId(paletteItems(c), 'needs-you').run!();
    expect(c.handlers.selectTab).toHaveBeenCalledWith(workspaces[1].tabs[0]);
    const calm = ctx({ infos: sessions.filter((s) => s.id !== 's2').map((s) => sessionInfo(s, 0)) });
    expect(ids(paletteItems(calm))).not.toContain('needs-you');
  });

  it('sets the theme, marking the current one, and shows the keys', () => {
    const c = ctx({ theme: 'dark' });
    const items = paletteItems(c);
    expect(['theme:system', 'theme:light', 'theme:dark'].map((id) => byId(items, id).label)).toEqual([
      'Theme: System', 'Theme: Light', 'Theme: Dark',
    ]);
    expect(byId(items, 'theme:dark').icon).toBe('check');
    expect(byId(items, 'theme:light').icon).toBe('theme');
    byId(items, 'theme:light').run!();
    expect(c.handlers.setTheme).toHaveBeenCalledWith('light');
    byId(items, 'keys').run!();
    expect(c.handlers.showKeys).toHaveBeenCalled();
    byId(items, 'run').run!();
    expect(c.handlers.openChooser).toHaveBeenCalled();
  });

  it('goes to Files (the phone rail has no Files item)', () => {
    const c = ctx();
    byId(paletteItems(c), 'view:files').run!();
    expect(c.handlers.showFiles).toHaveBeenCalled();
  });

  it('offers New ticket and Open ticket… (a ticket search step) when there are projects', () => {
    const c = ctx({ ticketProjectId: 'p1' });
    const items = paletteItems(c);
    byId(items, 'tickets:new').run!();
    expect(c.handlers.newTicket).toHaveBeenCalled();
    const open = byId(items, 'tickets:open');
    if (open.next?.kind !== 'tickets') throw new Error('expected a tickets step');
    expect(open.next.projectId).toBe('p1');
    open.next.run(summary);
    expect(c.handlers.openTicket).toHaveBeenCalledWith(summary);
    expect(ids(paletteItems(ctx({ projects: [] })))).not.toContain('tickets:new');
  });

  it('goes to tickets the API matched by key, matching their number too', () => {
    const c = ctx({ tickets: [summary] });
    const it = byId(paletteItems(c), `ticket:${summary.id}`);
    expect(it.label).toBe('PDT-42 · Fix login');
    expect(it.group).toBe(GO_TO);
    expect(it.keywords).toEqual(['PDT-42', '#42', '42']);
    it.run!();
    expect(c.handlers.openTicket).toHaveBeenCalledWith(summary);
  });

  it('labels a ticket tab with its key and title', () => {
    const w: Workspace[] = [{ ...workspaces[0], tabs: [{ id: 'tk', workspaceId: 'w1', position: 0, sessionId: null, runId: null, ticketId: summary.id }] }];
    const t = { ...summary, description: '', links: [], attempts: [], defaultBranch: 'pdt/PDT-42-fix-login', sessions: [], overlaps: [] };
    const items = paletteItems(ctx({ workspaces: w, infos: [ticketInfo(t, 0)] }));
    expect(byId(items, 'tab:tk').label).toBe('PDT-42 · Fix login');
  });

  it('starts the ticket on screen, and picks a ticket to start', () => {
    const w: Workspace[] = [{ ...workspaces[0], tabs: [{ id: 'tk', workspaceId: 'w1', position: 0, sessionId: null, runId: null, ticketId: summary.id }] }];
    const t = { ...summary, description: '', links: [], attempts: [], defaultBranch: 'pdt/PDT-42-fix-login', sessions: [], overlaps: [] };
    const c = ctx({ workspaces: w, tab: w[0].tabs[0], infos: [ticketInfo(t, 0)], ticketProjectId: 'p1' });
    const items = paletteItems(c);
    const here = byId(items, 'tickets:start-open');
    expect(here.label).toBe('Start PDT-42');
    here.run!();
    expect(c.handlers.startTicket).toHaveBeenCalledWith(t, true);
    const pick = byId(items, 'tickets:start');
    if (pick.next?.kind !== 'tickets') throw new Error('expected a tickets step');
    expect(pick.next.projectId).toBe('p1');
    pick.next.run(summary);
    expect(c.handlers.startTicket).toHaveBeenCalledWith(summary, false);
    // Not for a ticket already being worked on, nor without a ticket tab on screen.
    const busy = { ...t, stage: 'in_progress' as const };
    expect(ids(paletteItems(ctx({ workspaces: w, tab: w[0].tabs[0], infos: [ticketInfo(busy, 0)] })))).not.toContain('tickets:start-open');
    expect(ids(paletteItems(ctx()))).not.toContain('tickets:start-open');
  });
});

const summary: TicketSummary = {
  id: 'k42', projectId: 'p1', key: 'PDT-42', number: 42, title: 'Fix login', stage: 'ready', priority: 'high', labels: [],
  areas: [], rank: 'a', waiting: false, clearToStart: false, blockedBy: 0, attempt: null, overlap: 'none', overlapWith: [], createdAt: 0, updatedAt: 0, closedAt: null,
};
