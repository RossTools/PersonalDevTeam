import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError } from './client';

afterEach(() => vi.unstubAllGlobals());

function stubFetch(status: number, body: unknown) {
  // A fresh Response per call: a Response body can only be read once.
  const fetchMock = vi.fn().mockImplementation(
    async () => new Response(typeof body === 'string' ? body : JSON.stringify(body), { status }),
  );
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

describe('api client', () => {
  it('returns parsed JSON on success', async () => {
    stubFetch(200, [{ id: '1', name: 'p', repoPath: '/r', baseBranch: 'main', createdAt: 1 }]);
    expect((await api.listProjects())[0].name).toBe('p');
  });

  it('throws ApiError with server code and message', async () => {
    stubFetch(409, { code: 'conflict', message: 'a project for this repository already exists' });
    await expect(api.createProject({ name: 'x', repoPath: '/r' })).rejects.toMatchObject({
      status: 409, code: 'conflict', message: 'a project for this repository already exists',
    });
  });

  it('words a network failure instead of passing on the browser\'s text', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));
    const err = await api.health().catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 0, code: 'network_error', message: "Can't reach PDT: it isn't answering." });
  });

  it('lets an abort through as is', async () => {
    const abort = new DOMException('The operation was aborted.', 'AbortError');
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(abort));
    await expect(api.health()).rejects.toBe(abort);
  });

  it('handles non-JSON error bodies', async () => {
    stubFetch(502, 'Bad Gateway');
    const err = await api.health().catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('http_error');
    expect(err.message).toBe('HTTP 502');
  });

  it('returns undefined for 204 responses and encodes ids', async () => {
    const fetchMock = vi.fn().mockImplementation(async () => new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    await expect(api.deleteSession('a/b')).resolves.toBeUndefined();
    expect(fetchMock.mock.calls[0][0]).toBe('/api/sessions/a%2Fb');
    expect(fetchMock.mock.calls[0][1].method).toBe('DELETE');
  });

  it('posts new sessions as JSON', async () => {
    const fetchMock = stubFetch(200, { id: 's1' });
    await api.createSession({ projectId: 'p', kind: 'shell', cols: 80, rows: 24 });
    expect(fetchMock.mock.calls[0][0]).toBe('/api/sessions');
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ projectId: 'p', kind: 'shell', cols: 80, rows: 24 });
  });
});

describe('the UI token (PDT-204)', () => {
  it('goes with the user’s own changes, fetched again once after pdtd restarts', async () => {
    let token = 'first';
    const seen: (string | null)[] = [];
    const fetchMock = vi.fn().mockImplementation(async (path: string, init?: RequestInit) => {
      if (path === '/api/ui/token') return new Response(JSON.stringify({ token }), { status: 200 });
      const sent = new Headers(init?.headers).get('x-ui-token');
      seen.push(sent);
      if (sent !== token) return new Response(JSON.stringify({ code: 'stale_ui_token', message: 'stale' }), { status: 403 });
      return new Response(JSON.stringify({ count: 1 }), { status: 200 });
    });
    vi.stubGlobal('fetch', fetchMock);
    await api.bulkTickets({ ids: ['t'], action: 'stage', value: 'todo' });
    await api.bulkTickets({ ids: ['t'], action: 'stage', value: 'todo' });
    expect(seen).toEqual(['first', 'first']);
    expect(fetchMock.mock.calls.filter((c) => c[0] === '/api/ui/token')).toHaveLength(1);
    token = 'second';
    await api.bulkTickets({ ids: ['t'], action: 'stage', value: 'todo' });
    expect(seen.slice(2)).toEqual(['first', 'second']);
  });

  it('goes with new tickets and parts, so they are the user’s, and survives a pdtd restart (PDT-363)', async () => {
    let token = 'first';
    const seen: [string, string | null][] = [];
    const fetchMock = vi.fn().mockImplementation(async (path: string, init?: RequestInit) => {
      if (path === '/api/ui/token') return new Response(JSON.stringify({ token }), { status: 200 });
      const sent = new Headers(init?.headers).get('x-ui-token');
      seen.push([path, sent]);
      if (sent !== token) return new Response(JSON.stringify({ code: 'stale_ui_token', message: 'stale' }), { status: 403 });
      return new Response(JSON.stringify({ id: 't', items: [] }), { status: 201 });
    });
    vi.stubGlobal('fetch', fetchMock);
    // Whatever token an earlier test left cached, each goes through with the current one.
    await api.createTicket('p', { title: 'Quick one' });
    expect(seen.at(-1)).toEqual(['/api/projects/p/tickets', 'first']);
    token = 'second';
    seen.length = 0;
    await api.createParts('t', { parts: [{ title: 'One' }] });
    expect(seen).toEqual([['/api/tickets/t/parts', 'first'], ['/api/tickets/t/parts', 'second']]);
  });
});

describe('the area modules', () => {
  it('are all in `api`, with no call name used by two areas', () => {
    // Spreading two areas with the same key would silently keep only the last one.
    const modules = import.meta.glob<Record<string, unknown>>(['./*.ts', '!./client.ts', '!./core.ts', '!./*.test.ts'], { eager: true });
    const names = Object.values(modules).flatMap((m) =>
      Object.entries(m).filter(([k]) => k.endsWith('Api')).flatMap(([, area]) => Object.keys(area as object)));
    expect(names.length).toBeGreaterThan(200);
    expect(names.filter((n, i) => names.indexOf(n) !== i)).toEqual([]);
    expect(Object.keys(api).sort()).toEqual([...names].sort());
  });
});
