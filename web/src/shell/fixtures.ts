import { vi } from 'vitest';
import { githubLinks, stubApi, type Call } from '../test-utils';

// Fixtures the AppShell test files share (AppShell.test.tsx and AppShell.<area>.test.tsx).

export const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'ALP', forge: githubLinks('o/alpha') };
export const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, bypassPermissions: false, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: 1, activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'terminal', model: null, permissionMode: null };

// The shell always renders the update banner; a fully set-up host shows none.
const hostTool = { found: true, ok: true, detail: 'ok' };
export const hostRoutes = {
  'GET /api/host/settings': {
    settings: { remoteAccess: true, ownerLogin: 'me@example.com', updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 }, maxConcurrentRuns: 4, agentCredentials: 'subscription', defaultAgent: 'claude', defaultSessionMode: 'conversation' },
    ownerFromEnv: null,
    tokenPresent: true,
    tokenError: null,
    remote: { enabled: true, active: true, url: 'https://box.ts.net', dnsName: 'box.ts.net', error: null, tailscaleLogin: 'me@example.com' },
  },
  'GET /api/host/update': {
    current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
    error: null, token: { state: 'valid', expiresAt: null }, lastResult: null,
  },
  'GET /api/host/checks': { claude: hostTool, codex: hostTool, devin: hostTool, git: hostTool, tailscale: hostTool, checkedAt: 1 },
  'GET /api/host/whoami': { caller: 'local', login: null },
};

export const health = (dev: boolean, port: number) => ({
  status: 'ok', name: 'Personal Dev Team', version: '0.1.0-local', profile: dev ? 'dev' : 'stable', port, dev,
});

/** Below the md breakpoint: every max-width media query matches (the phone layout). */
export const stubPhone = () =>
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('max-width'),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));

export function stubAlpha() {
  stubApi({
    ...hostRoutes,
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: [shell],
    [`GET /api/projects/${project.id}/runs`]: [],
    'GET /api/workspaces': [{ id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] }],
  });
}

export const sess = (id: string, title: string, extra: Record<string, unknown> = {}) => ({ ...shell, id, title, ...extra });
export const tabOf = (id: string, workspaceId: string, sessionId: string, position: number) => ({ id, workspaceId, sessionId, runId: null, position });

/** Alpha with sessions a, b (tabs in Alpha's workspace) and c (no tab), and a free-form workspace "Release". */
export function stubWorkspaces(extra: Record<string, unknown> = {}): Call[] {
  localStorage.clear();
  const sessions = [
    sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude', activity: 'working' }),
    sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', activity: 'needs_you', unseen: true }),
    sess('c', 'Shell C'),
    sess('d', 'Claude D', { kind: 'agent_tui', agent: 'claude', activity: 'idle', unseen: true }),
  ];
  const workspaces = [
    { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [tabOf('ta', 'wa', 'a', 0), tabOf('tb', 'wa', 'b', 1)] },
    { id: 'wr', name: 'Release', projectId: null, position: 1, createdAt: 2, tabs: [] },
  ];
  return stubApi({
    ...hostRoutes,
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: sessions,
    [`GET /api/projects/${project.id}/runs`]: [],
    'GET /api/workspaces': workspaces,
    ...extra,
  });
}

export const runConfig = {
  filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null,
  effective: {
    runs: [{ name: 'web', setup: null, command: 'x', web: true, data: true, env: {} }],
    tests: [{ name: 'check', setup: null, command: 'y', web: false, data: false, env: {} }],
  },
};
export const runOf = (id: string) => ({
  id, projectId: project.id, worktreeId: null, branch: 'main', name: 'web', kind: 'run', state: 'starting',
  port: 8401, url: null, dataDir: '/d', cwd: '/r', exitCode: null, error: null, detached: false, createdAt: 1, updatedAt: 1,
});
