import { describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Run } from '../api/gen/Run';
import type { Session } from '../api/gen/Session';
import type { Tab } from '../api/gen/Tab';
import type { TicketSummary } from '../api/gen/TicketSummary';
import type { Workspace } from '../api/gen/Workspace';
import { runInfo, sessionInfo, ticketInfo } from '../workspaces/useWorkspaces';
import { ACTIONS, FILES, GO_TO, fileCheckouts, paletteItems, type PaletteContext, type PaletteHandlers, type PaletteItem } from './paletteItems';

const project = (id: string, name: string, key: string): Project => ({
  id, name, key, repoPath: `/r/${id}`, baseBranch: 'main', createdAt: 0, forge: null, kind: 'repo',
});

const session = (over: Partial<Session> & { id: string }): Session => ({
  projectId: 'p1', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: over.id,
  cwd: '/r/p1', status: 'running', exitCode: null, createdAt: 0, endedAt: null, externalSessionId: null,
  freshStartedAt: null, activity: 'idle', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'terminal', model: null, permissionMode: null, persona: null, background: [], ...over,
});

const run: Run = {
  id: 'r1', projectId: 'p1', worktreeId: 'wt1', branch: 'feature-x', name: 'pdt', kind: 'run', state: 'ready', port: 8500,
  url: null, dataDir: null, cwd: '/r/p1/.worktrees/feature-x', exitCode: null, error: null, detached: false, createdAt: 0, updatedAt: 0,
};

const tab = (id: string, workspaceId: string, position: number, item: { sessionId?: string; runId?: string }): Tab => ({
  id, workspaceId, position, sessionId: item.sessionId ?? null, runId: item.runId ?? null, ticketId: null, worktreeId: null,
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
    moveTabToNewWorkspace: vi.fn(), setTheme: vi.fn(), showKeys: vi.fn(), reportBug: vi.fn(), showFiles: vi.fn(), showWorkflows: vi.fn(), showInbox: vi.fn(), showPersonas: vi.fn(), showPm: vi.fn(), showNotes: vi.fn(), addProject: vi.fn(), newTicket: vi.fn(), newIssue: vi.fn(), openTicket: vi.fn(), startTicket: vi.fn(), startNextPart: vi.fn(), openFile: vi.fn(),
  };
}

function ctx(over: Partial<PaletteContext> = {}): PaletteContext {
  return {
    workspaces,
    infos: [...sessions.map((s) => sessionInfo(s)), runInfo(run)],
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
    byId(items, 'new:devin-terminal').run!();
    expect(c.handlers.newSession).toHaveBeenCalledWith('devin_terminal', { projectId: 'p1', worktreeId: 'wt1' });
    expect(ids(paletteItems(ctx({ place: null }))).some((i) => i.startsWith('new:'))).toBe(false);
  });

  it('starts a Devin conversation or terminal', () => {
    const c = ctx();
    const items = paletteItems(c);
    expect(byId(items, 'new:devin').label).toBe('New Devin here');
    byId(items, 'new:devin').run!();
    expect(c.handlers.newSession).toHaveBeenCalledWith('devin', { projectId: 'p1', worktreeId: 'wt1' });
  });

  it("starts the host's ACP agents as conversations, after the built-in agents", () => {
    const c = ctx({ acpAgents: [{ agent: 'acp:fake', label: 'Fake' }], agents: (a) => (a === 'acp:gone' ? 'missing' : 'ready') });
    const items = paletteItems(c);
    expect(byId(items, 'new:acp:fake').label).toBe('New Fake here');
    expect(byId(items, 'new:acp:fake').icon).toBe('acp');
    expect(ids(items).filter((i) => i.startsWith('new:')).slice(-2)).toEqual(['new:acp:fake', 'new:shell']);
    byId(items, 'new:acp:fake').run!();
    expect(c.handlers.newSession).toHaveBeenCalledWith('acp:fake', { projectId: 'p1', worktreeId: 'wt1' });
    const missing = paletteItems(ctx({ acpAgents: [{ agent: 'acp:gone', label: 'Gone' }], agents: (a) => (a === 'acp:gone' ? 'missing' : 'ready') }));
    expect(ids(missing)).not.toContain('new:acp:gone');
  });

  it('leaves out agents the host lacks and marks a logged-out one', () => {
    const items = paletteItems(ctx({ agents: (a) => (a === 'devin' ? 'missing' : a === 'codex' ? 'logged_out' : 'ready') }));
    expect(ids(items).some((i) => i.startsWith('new:devin'))).toBe(false);
    expect(byId(items, 'new:codex').detail).toBe('not logged in');
    expect(byId(items, 'new:codex-terminal').detail).toBe('not logged in');
    expect(byId(items, 'new:claude').detail).toBeUndefined();
    expect(ids(items)).toContain('new:shell');
  });

  it("lists the host's default agent and view first", () => {
    const items = paletteItems(ctx({ defaults: { agent: 'codex', mode: 'terminal' } }));
    expect(ids(items).filter((i) => i.startsWith('new:'))).toEqual([
      'new:codex-terminal', 'new:codex', 'new:claude-terminal', 'new:claude', 'new:devin-terminal', 'new:devin', 'new:shell',
    ]);
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
    const calm = ctx({ infos: sessions.filter((s) => s.id !== 's2').map((s) => sessionInfo(s)) });
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

  it('goes to the Notes, with its key', () => {
    const c = ctx();
    const notes = byId(paletteItems(c), 'view:notes');
    expect(notes.shortcut).toBe('Ctrl+] o');
    notes.run!();
    expect(c.handlers.showNotes).toHaveBeenCalled();
  });

  it('goes to the Inbox, Personas and the PM', () => {
    const c = ctx();
    byId(paletteItems(c), 'view:inbox').run!();
    expect(c.handlers.showInbox).toHaveBeenCalled();
    byId(paletteItems(c), 'view:personas').run!();
    expect(c.handlers.showPersonas).toHaveBeenCalled();
    byId(paletteItems(c), 'view:pm').run!();
    expect(c.handlers.showPm).toHaveBeenCalled();
  });

  it('offers Add project… with or without projects', () => {
    for (const c of [ctx(), ctx({ projects: [] })]) {
      byId(paletteItems(c), 'projects:add').run!();
      expect(c.handlers.addProject).toHaveBeenCalled();
    }
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

  it('offers New issue in the current project\'s repository when its forge takes issues (PDT-221)', () => {
    expect(ids(paletteItems(ctx()))).not.toContain('issues:new');
    const c = ctx({ issueRepo: 'o/r' });
    const item = byId(paletteItems(c), 'issues:new');
    expect([item.label, item.detail]).toEqual(['New issue', 'o/r']);
    item.run!();
    expect(c.handlers.newIssue).toHaveBeenCalled();
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
    const w: Workspace[] = [{ ...workspaces[0], tabs: [{ id: 'tk', workspaceId: 'w1', position: 0, sessionId: null, runId: null, ticketId: summary.id, worktreeId: null }] }];
    const t = { ...summary, description: '', links: [], attempts: [], defaultBranch: 'pdt/PDT-42-fix-login', sessions: [], overlaps: [], partList: [], webLinks: [], createdBy: null };
    const items = paletteItems(ctx({ workspaces: w, infos: [ticketInfo(t)] }));
    expect(byId(items, 'tab:tk').label).toBe('PDT-42 · Fix login');
  });

  it('starts the ticket on screen, and picks a ticket to start', () => {
    const w: Workspace[] = [{ ...workspaces[0], tabs: [{ id: 'tk', workspaceId: 'w1', position: 0, sessionId: null, runId: null, ticketId: summary.id, worktreeId: null }] }];
    const t = { ...summary, description: '', links: [], attempts: [], defaultBranch: 'pdt/PDT-42-fix-login', sessions: [], overlaps: [], partList: [], webLinks: [], createdBy: null };
    const c = ctx({ workspaces: w, tab: w[0].tabs[0], infos: [ticketInfo(t)], ticketProjectId: 'p1' });
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
    expect(ids(paletteItems(ctx({ workspaces: w, tab: w[0].tabs[0], infos: [ticketInfo(busy)] })))).not.toContain('tickets:start-open');
    expect(ids(paletteItems(ctx()))).not.toContain('tickets:start-open');
  });

  it('starts the next part of a parent on screen, never the parent itself', () => {
    const w: Workspace[] = [{ ...workspaces[0], tabs: [{ id: 'tk', workspaceId: 'w1', position: 0, sessionId: null, runId: null, ticketId: summary.id, worktreeId: null }] }];
    const parts = { total: 2, done: 0, merged: 0, review: 0, progress: 0, todo: 2, backlog: 0, segments: ['todo' as const, 'todo' as const] };
    const t = { ...summary, parts, description: '', links: [], attempts: [], defaultBranch: 'pdt/PDT-42-fix-login', sessions: [], overlaps: [], partList: [], webLinks: [], createdBy: null };
    const c = ctx({ workspaces: w, tab: w[0].tabs[0], infos: [ticketInfo(t)] });
    const items = paletteItems(c);
    expect(ids(items)).not.toContain('tickets:start-open');
    const next = byId(items, 'tickets:start-next-part');
    expect(next.label).toBe('Start next part of PDT-42');
    next.run!();
    expect(c.handlers.startNextPart).toHaveBeenCalledWith(t);
    const closed = { ...t, stage: 'done' as const };
    expect(ids(paletteItems(ctx({ workspaces: w, tab: w[0].tabs[0], infos: [ticketInfo(closed)] })))).not.toContain('tickets:start-next-part');
  });

  it('lists found files last, below every other group, and opens them', () => {
    const checkout = { projectId: 'p1', worktreeId: 'wt1', label: 'PDT-42' };
    const found = [{ checkout, path: 'web/src/palette/CommandPalette.tsx' }, { checkout, path: 'README.md' }];
    const c = ctx({ tickets: [summary], files: { query: 'pal', found } });
    const items = paletteItems(c);
    const groups = items.map((i) => i.group);
    expect(groups.indexOf(FILES)).toBe(groups.length - 2);
    expect(groups.lastIndexOf(ACTIONS)).toBeLessThan(groups.indexOf(FILES));
    const [file, readme] = items.slice(-2);
    expect([file.label, file.detail, file.icon]).toEqual(['CommandPalette.tsx', 'web/src/palette · PDT-42', 'file']);
    expect(readme.detail).toBe('PDT-42');
    expect(file.keywords).toContain('pal');
    file.run!();
    expect(c.handlers.openFile).toHaveBeenCalledWith(found[0]);
    expect(ids(paletteItems(ctx())).some((id) => id.startsWith('file:'))).toBe(false);
  });

  it('finds files in the worktree in context, else the project, else every project', () => {
    const projects = [project('p1', 'PersonalDevTeam', 'PDT'), project('p2', 'Demo', 'DEMO'), { ...project('p3', 'Notes', 'NOTE'), kind: 'personal' as const }];
    const infos = ctx().infos;
    expect(fileCheckouts({ projects, infos, place: { projectId: 'p1', worktreeId: 'wt1' } })).toEqual([{ projectId: 'p1', worktreeId: 'wt1', label: 'fix-login' }]);
    const withTicket = [sessionInfo(session({ id: 's9', worktreeId: 'wt1', ticket: { id: 'k42', key: 'PDT-42' } })), ...infos];
    expect(fileCheckouts({ projects, infos: withTicket, place: { projectId: 'p1', worktreeId: 'wt1' } })[0].label).toBe('PDT-42');
    expect(fileCheckouts({ projects, infos, place: { projectId: 'p1', worktreeId: null } })).toEqual([{ projectId: 'p1', worktreeId: null, label: 'PersonalDevTeam' }]);
    expect(fileCheckouts({ projects, infos, place: null })).toEqual([
      { projectId: 'p1', worktreeId: null, label: 'PersonalDevTeam' },
      { projectId: 'p2', worktreeId: null, label: 'Demo' },
    ]);
    expect(fileCheckouts({ projects, infos, place: { projectId: 'p3', worktreeId: null } })).toEqual([]);
  });
});

const summary: TicketSummary = {
  id: 'k42', projectId: 'p1', key: 'PDT-42', number: 42, title: 'Fix login', stage: 'ready', priority: 'high', labels: [],
  areas: [], rank: 'a', waiting: false, clearToStart: false, blockedBy: 0, attempt: null, overlap: 'none', overlapWith: [], createdAt: 0, updatedAt: 0, closedAt: null,
  parent: null, parts: null, pipelineStage: null, agent: null, autoStartAt: null, pickupHold: null, pickupPaused: null, pickupFailed: null, external: [], source: 'user', admitted: false, createdBy: null, proposed: false,
};
