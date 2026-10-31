import { describe, expect, it } from 'vitest';
import { isDetail, isOldPath, parseLocation, parseRoute, routePath, sectionSlug, upRoute, viewRoute, type Route } from './route';
import { VIEWS } from './views';

describe('routes', () => {
  // Each path in the ticket, the route it reads as, and back.
  const cases: [string, Route][] = [
    ['/pm', { view: 'pm', pm: 'current' }],
    ['/pm/s1', { view: 'pm', pm: 's1' }],
    ['/pm/earlier', { view: 'pm', pm: 'earlier' }],
    ['/sessions', { view: 'sessions', workspaceId: null, tabId: null }],
    ['/sessions/w1', { view: 'sessions', workspaceId: 'w1', tabId: null }],
    ['/sessions/w1/t1', { view: 'sessions', workspaceId: 'w1', tabId: 't1' }],
    ['/inbox', { view: 'inbox', itemId: null }],
    ['/inbox/i1', { view: 'inbox', itemId: 'i1' }],
    ['/tickets', { view: 'tickets', board: false, ticket: null }],
    ['/tickets?view=board', { view: 'tickets', board: true, ticket: null }],
    ['/tickets/PDT-12', { view: 'tickets', board: false, ticket: 'PDT-12' }],
    ['/tickets/PDT-12?view=board', { view: 'tickets', board: true, ticket: 'PDT-12' }],
    ['/notes', { view: 'notes', memory: false, note: null }],
    ['/notes/pm', { view: 'notes', memory: true, note: null }],
    ['/notes/n1', { view: 'notes', memory: false, note: 'n1' }],
    ['/notes/new', { view: 'notes', memory: false, note: 'new' }],
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
    // A folder (the phone's drill-down) ends in `/`.
    ['/files/src/', { view: 'files', path: 'src/', project: null, worktree: null }],
    ['/files/a%20b/c/?project=PDT', { view: 'files', path: 'a b/c/', project: 'PDT', worktree: null }],
    ['/files/a%20b/c%23d.md?project=PDT&worktree=wt', { view: 'files', path: 'a b/c#d.md', project: 'PDT', worktree: 'wt' }],
    ['/setup', { view: 'setup' }],
    ['/settings', { view: 'settings', page: { kind: 'index' } }],
    ['/settings/host', { view: 'settings', page: { kind: 'host', section: null } }],
    ['/settings/host/updates', { view: 'settings', page: { kind: 'host', section: 'updates' } }],
    ['/settings/connections', { view: 'settings', page: { kind: 'connections' } }],
    ['/settings/plugins', { view: 'settings', page: { kind: 'plugins', id: null } }],
    ['/settings/plugins/jira', { view: 'settings', page: { kind: 'plugins', id: 'jira' } }],
    ['/settings/secrets', { view: 'settings', page: { kind: 'secrets' } }],
    ['/settings/mcp-servers', { view: 'settings', page: { kind: 'mcp-servers' } }],
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
    // PDT-228: the old Integrations page and This host's GitHub section are the plugins'.
    expect(parseRoute('/settings/integrations')).toEqual({ view: 'settings', page: { kind: 'plugins', id: null } });
    expect(parseRoute('/settings/host/github')).toEqual({ view: 'settings', page: { kind: 'plugins', id: 'github' } });
    expect(isOldPath('/settings/integrations') && isOldPath('/settings/host/github') && !isOldPath('/settings/host/updates')).toBe(true);
    expect(parseRoute('/settings/nonsense')).toEqual({ view: 'settings', page: { kind: 'index' } });
    expect(parseRoute('/workflows/wf/nonsense')).toEqual({ view: 'workflows', target: { kind: 'workflow', id: 'wf' } });
    expect(parseRoute('/nowhere/at/all')).toEqual({ view: null });
    expect(parseRoute('/sessions/w1/t1/')).toEqual({ view: 'sessions', workspaceId: 'w1', tabId: 't1' });
    expect(parseRoute('/pm/%E0%A4%A')).toEqual({ view: 'pm', pm: '%E0%A4%A' });
  });

  it("reads the notes' old paths as the Notes view's (ticket descriptions link to them)", () => {
    expect(parseRoute('/pm/notes/n1')).toEqual({ view: 'notes', memory: false, note: 'n1' });
    expect(routePath(parseRoute('/pm/notes/n1'))).toBe('/notes/n1');
    expect(parseRoute('/pm/notes')).toEqual({ view: 'notes', memory: true, note: null });
    expect(routePath(parseRoute('/pm/notes'))).toBe('/notes/pm');
  });

  it("gives every view its own path", () => {
    for (const v of VIEWS) expect(parseLocation(routePath(viewRoute(v.id))).view).toBe(v.id);
  });

  it('tells detail screens from top-level ones, and where each goes up to', () => {
    const top = ['/pm', '/notes', '/notes/pm', '/sessions', '/inbox', '/tickets?view=board', '/workflows', '/personas', '/files?project=PDT', '/settings', '/'];
    for (const p of top) expect(isDetail(parseLocation(p)), p).toBe(false);
    const up: [string, string][] = [
      ['/notes/n1', '/notes'], ['/notes/new', '/notes'], ['/pm/earlier', '/pm'], ['/pm/s1', '/pm'], ['/pm/new', '/pm'], ['/sessions/w/t', '/sessions'], ['/inbox/i', '/inbox'], ['/tickets/PDT-1?view=board', '/tickets?view=board'],
      ['/workflows/wf/runs/r', '/workflows/wf/runs'], ['/workflows/wf/runs', '/workflows'], ['/workflows/templates/tp', '/workflows'],
      ['/personas/p', '/personas'],
      ['/files/a/b.md?project=PDT', '/files/a/?project=PDT'], ['/files/a/b/', '/files/a/'], ['/files/a/', '/files'], ['/files/b.md', '/files'],
      // A Settings section goes up to its page, the page to the list of them.
      ['/settings/projects/PDT/danger-zone', '/settings/projects/PDT'], ['/settings/projects/PDT', '/settings'],
      ['/settings/host/updates', '/settings/host'], ['/settings/host', '/settings'], ['/settings/notifications', '/settings'],
      // A plugin's page goes up to the catalog.
      ['/settings/plugins/jira', '/settings/plugins'], ['/settings/plugins', '/settings'],
    ];
    for (const [p, to] of up) {
      expect(isDetail(parseLocation(p)), p).toBe(true);
      expect(routePath(upRoute(parseLocation(p))), p).toBe(to);
    }
  });

  it('names sections by their heading', () => {
    expect(sectionSlug('Danger zone')).toBe('danger-zone');
    expect(sectionSlug('Remote access')).toBe('remote-access');
  });
});
