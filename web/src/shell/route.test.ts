import { describe, expect, it } from 'vitest';
import { parseLocation, parseRoute, routePath, sectionSlug, viewRoute, type Route } from './route';
import { VIEWS } from './views';

describe('routes', () => {
  // Each path in the ticket, the route it reads as, and back.
  const cases: [string, Route][] = [
    ['/pm', { view: 'pm', pm: 'current' }],
    ['/pm/s1', { view: 'pm', pm: 's1' }],
    ['/pm/notes', { view: 'pm', pm: 'notes' }],
    ['/sessions', { view: 'sessions', workspaceId: null, tabId: null }],
    ['/sessions/w1', { view: 'sessions', workspaceId: 'w1', tabId: null }],
    ['/sessions/w1/t1', { view: 'sessions', workspaceId: 'w1', tabId: 't1' }],
    ['/inbox', { view: 'inbox', itemId: null }],
    ['/inbox/i1', { view: 'inbox', itemId: 'i1' }],
    ['/tickets', { view: 'tickets', board: false, ticket: null }],
    ['/tickets?view=board', { view: 'tickets', board: true, ticket: null }],
    ['/tickets/PDT-12', { view: 'tickets', board: false, ticket: 'PDT-12' }],
    ['/tickets/PDT-12?view=board', { view: 'tickets', board: true, ticket: 'PDT-12' }],
    ['/workflows', { view: 'workflows', target: null }],
    ['/workflows/wf', { view: 'workflows', target: { kind: 'workflow', id: 'wf' } }],
    ['/workflows/wf/outline', { view: 'workflows', target: { kind: 'workflow', id: 'wf', tab: 'outline' } }],
    ['/workflows/wf/runs', { view: 'workflows', target: { kind: 'workflow', id: 'wf', tab: 'runs' } }],
    ['/workflows/wf/graph', { view: 'workflows', target: { kind: 'workflow', id: 'wf', tab: 'graph' } }],
    ['/workflows/wf/text', { view: 'workflows', target: { kind: 'workflow', id: 'wf', tab: 'text' } }],
    ['/workflows/wf/versions', { view: 'workflows', target: { kind: 'workflow', id: 'wf', tab: 'versions' } }],
    ['/workflows/wf/runs/r1', { view: 'workflows', target: { kind: 'workflow', id: 'wf', tab: 'runs', runId: 'r1' } }],
    ['/workflows/templates/tp', { view: 'workflows', target: { kind: 'template', id: 'tp' } }],
    ['/personas', { view: 'personas', personaId: null }],
    ['/personas/p1', { view: 'personas', personaId: 'p1' }],
    ['/files', { view: 'files', path: null, project: null, worktree: null }],
    ['/files/src/main.rs', { view: 'files', path: 'src/main.rs', project: null, worktree: null }],
    ['/files/a%20b/c%23d.md?project=PDT&worktree=wt', { view: 'files', path: 'a b/c#d.md', project: 'PDT', worktree: 'wt' }],
    ['/settings/host', { view: 'settings', page: { kind: 'host', section: null } }],
    ['/settings/host/updates', { view: 'settings', page: { kind: 'host', section: 'updates' } }],
    ['/settings/connections', { view: 'settings', page: { kind: 'connections' } }],
    ['/settings/notifications', { view: 'settings', page: { kind: 'notifications' } }],
    ['/settings/projects/PDT', { view: 'settings', page: { kind: 'project', key: 'PDT', section: null } }],
    ['/settings/projects/PDT/worktrees', { view: 'settings', page: { kind: 'project', key: 'PDT', section: 'worktrees' } }],
    ['/', { view: null }],
  ];

  it.each(cases)('%s', (path, route) => {
    expect(parseLocation(path)).toEqual(route);
    expect(routePath(route)).toBe(path);
  });

  it('reads paths the app writes no other way', () => {
    expect(parseRoute('/settings')).toEqual({ view: 'settings', page: { kind: 'host', section: null } });
    expect(parseRoute('/workflows/wf/nonsense')).toEqual({ view: 'workflows', target: { kind: 'workflow', id: 'wf' } });
    expect(parseRoute('/nowhere/at/all')).toEqual({ view: null });
    expect(parseRoute('/sessions/w1/t1/')).toEqual({ view: 'sessions', workspaceId: 'w1', tabId: 't1' });
    expect(parseRoute('/pm/%E0%A4%A')).toEqual({ view: 'pm', pm: '%E0%A4%A' });
  });

  it("gives every view its own path", () => {
    for (const v of VIEWS) expect(parseLocation(routePath(viewRoute(v.id))).view).toBe(v.id);
  });

  it('names sections by their heading', () => {
    expect(sectionSlug('Danger zone')).toBe('danger-zone');
    expect(sectionSlug('Remote access')).toBe('remote-access');
  });
});
