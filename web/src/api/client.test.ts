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

  it('handles non-JSON error bodies', async () => {
    stubFetch(502, 'Bad Gateway');
    const err = await api.health().catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('http_error');
    expect(err.message).toBe('HTTP 502');
  });
});
