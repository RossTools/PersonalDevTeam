import { render } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { vi } from 'vitest';
import { ConfirmProvider, Toaster } from '@ui';

export function renderWithQuery(ui: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  // The same providers as main.tsx: confirm dialogs and toasts.
  const wrap = (node: ReactNode) => (
    <QueryClientProvider client={qc}>
      <ConfirmProvider>
        {node}
        <Toaster />
      </ConfirmProvider>
    </QueryClientProvider>
  );
  const result = render(wrap(ui));
  return { ...result, qc, rerender: (node: ReactNode) => result.rerender(wrap(node)) };
}

/** A stub route value that answers with a specific status and JSON body. */
export class Reply {
  constructor(public status: number, public body: unknown) {}
}

export interface Call { method: string; path: string; body: unknown }
type Route = ((call: Call) => unknown) | object | string | number | boolean | null | undefined;

/**
 * Stubs fetch with `"METHOD /path"` routes. `undefined` responds 204; unknown routes respond 404. A request with a query
 * string matches its exact route first, then the route without the query (a function route can read `call.path`).
 */
export function stubApi(routes: Record<string, Route>): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = {
      method: (init?.method ?? 'GET').toUpperCase(),
      path: String(input),
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    };
    calls.push(call);
    const exact = `${call.method} ${call.path}`;
    const key = exact in routes ? exact : `${call.method} ${call.path.split('?')[0]}`;
    if (!(key in routes)) return new Response(JSON.stringify({ code: 'not_found', message: exact }), { status: 404 });
    const route = routes[key];
    // A function route may return a promise, to hold the response until the test resolves it.
    const value = await (typeof route === 'function' ? (route as (c: Call) => unknown)(call) : route);
    if (value instanceof Reply) return new Response(JSON.stringify(value.body), { status: value.status });
    if (value instanceof Response) return value;
    return value === undefined ? new Response(null, { status: 204 }) : new Response(JSON.stringify(value), { status: 200 });
  }));
  return calls;
}
