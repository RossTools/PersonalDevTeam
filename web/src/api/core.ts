import type { ApiErrorBody } from './gen/ApiErrorBody';

// The client's shared plumbing: every area module (projects.ts, tickets.ts, ...) calls the server through these.

export class ApiError extends Error {
  /** `data`: what the client needs to act on the error, for the few that carry it (`untrusted_commands`). */
  constructor(public status: number, public code: string, message: string, public data?: unknown) {
    super(message);
    this.name = 'ApiError';
  }
}

export async function request<T>(path: string, init?: RequestInit, opts?: { asUser?: boolean }): Promise<T> {
  const res = opts?.asUser ? await sendAsUser(path, init) : await send(path, init);
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

/** A plain-text response (PDT's guides are Markdown). */
export async function requestText(path: string): Promise<string> {
  return (await send(path)).text();
}

export async function send(path: string, init?: RequestInit): Promise<Response> {
  const res = await fetch(path, { ...init, headers: { 'content-type': 'application/json', ...init?.headers } });
  if (!res.ok) throw await errorOf(res);
  return res;
}

async function errorOf(res: Response): Promise<ApiError> {
  let body: Partial<ApiErrorBody> = {};
  try {
    body = await res.json();
  } catch {
    // non-JSON error body (proxy, crash page)
  }
  return new ApiError(res.status, body.code ?? 'http_error', body.message ?? `HTTP ${res.status}`, body.data);
}

/** This pdtd's UI token (PDT-204), fetched once; it changes when pdtd restarts. */
let uiToken: Promise<string> | null = null;

function fetchUiToken(): Promise<string> {
  uiToken ??= send('/api/ui/token')
    .then((r) => r.json())
    .then((b: { token: string }) => b.token)
    .catch((e: unknown) => {
      uiToken = null;
      throw e;
    });
  return uiToken;
}

/**
 * A change only the user may make (letting an external ticket in, PDT-204): sent with the UI token, which tells the
 * user's click from an agent's curl. A token from before a pdtd restart is fetched again and the change retried once.
 */
async function sendAsUser(path: string, init?: RequestInit): Promise<Response> {
  for (let attempt = 0; ; attempt++) {
    const token = await fetchUiToken();
    try {
      return await send(path, { ...init, headers: { ...init?.headers, 'x-ui-token': token } });
    } catch (e) {
      if (attempt > 0 || !(e instanceof ApiError) || e.code !== 'stale_ui_token') throw e;
      uiToken = null;
    }
  }
}

export const enc = encodeURIComponent;

/** `?a=1&b=x` from the defined, non-empty fields (empty string when there are none). */
export function queryString(q: object): string {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(q)) {
    if (v !== undefined && v !== null && v !== '') params.set(k, String(v));
  }
  const s = params.toString();
  return s ? `?${s}` : '';
}
