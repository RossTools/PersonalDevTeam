import { render } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { vi } from 'vitest';

export function renderWithQuery(ui: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrap = (node: ReactNode) => <QueryClientProvider client={qc}>{node}</QueryClientProvider>;
  const result = render(wrap(ui));
  return { ...result, rerender: (node: ReactNode) => result.rerender(wrap(node)) };
}

/** A stub route value that answers with a specific status and JSON body. */
export class Reply {
  constructor(public status: number, public body: unknown) {}
}

export interface Call { method: string; path: string; body: unknown }
type Route = ((call: Call) => unknown) | object | string | number | boolean | null | undefined;

/** Stubs fetch with `"METHOD /path"` routes. `undefined` responds 204; unknown routes respond 404. */
export function stubApi(routes: Record<string, Route>): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = {
      method: (init?.method ?? 'GET').toUpperCase(),
      path: String(input),
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    };
    calls.push(call);
    const key = `${call.method} ${call.path}`;
    if (!(key in routes)) return new Response(JSON.stringify({ code: 'not_found', message: key }), { status: 404 });
    const route = routes[key];
    // A function route may return a promise, to hold the response until the test resolves it.
    const value = await (typeof route === 'function' ? (route as (c: Call) => unknown)(call) : route);
    if (value instanceof Reply) return new Response(JSON.stringify(value.body), { status: value.status });
    return value === undefined ? new Response(null, { status: 204 }) : new Response(JSON.stringify(value), { status: 200 });
  }));
  return calls;
}
