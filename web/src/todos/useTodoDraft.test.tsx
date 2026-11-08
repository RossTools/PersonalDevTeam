import { QueryClientProvider } from '@tanstack/react-query';
import { act, render, renderHook, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Toaster } from '@ui';
import type { Todo } from '../api/gen/Todo';
import { createQueryClient } from '../common/queryClient';
import { AUTOSAVE_MS } from '../notes/useNoteDraft';
import { Reply, stubApi, type Call } from '../test-utils';
import { TODOS_KEY } from './useTodos';
import { useTodoDraft } from './useTodoDraft';

const todo = (more: Partial<Todo> = {}): Todo => ({
  id: 'a', title: 'Buy milk', position: 1, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
  doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, body: '', dueOn: null, dueTime: null, recurrence: null, ...more,
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** The hook over a todo the test changes (as the list's poll would), with the query client it writes to. */
function setup(initial: Todo = todo()) {
  const qc = createQueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData(TODOS_KEY, [initial]);
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  const hook = renderHook(({ t }: { t: Todo | undefined }) => useTodoDraft(t), { wrapper, initialProps: { t: initial } });
  return { ...hook, qc };
}

const patches = (calls: Call[]) => calls.filter((c) => c.method === 'PATCH').map((c) => c.body);
const echo = (c: Call) => todo(c.body as Partial<Todo>);

describe('useTodoDraft', () => {
  it('saves only what changed, a moment after typing stops, and updates the list', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const calls = stubApi({ 'PATCH /api/todos/a': echo });
    const { result, qc } = setup();
    act(() => result.current.setBody('2 litres'));
    expect(result.current.state).toBe('pending');
    act(() => vi.advanceTimersByTime(AUTOSAVE_MS - 50));
    expect(patches(calls)).toEqual([]);
    await act(async () => vi.advanceTimersByTime(100));
    expect(patches(calls)).toEqual([{ body: '2 litres' }]);
    await waitFor(() => expect(result.current.state).toBe('saved'));
    expect(qc.getQueryData<Todo[]>(TODOS_KEY)?.[0].body).toBe('2 litres');
  });

  it('never saves an empty title: the last one stays until a title is typed', async () => {
    const calls = stubApi({ 'PATCH /api/todos/a': echo });
    const { result } = setup();
    act(() => result.current.setTitle('  '));
    await act(() => result.current.flush());
    expect(patches(calls)).toEqual([]);
    // With the body changed too, only the body goes.
    act(() => result.current.setBody('note'));
    await act(() => result.current.flush());
    expect(patches(calls)).toEqual([{ body: 'note' }]);
    act(() => result.current.setTitle('Buy oat milk'));
    await act(() => result.current.flush());
    expect(patches(calls)).toEqual([{ body: 'note' }, { title: 'Buy oat milk' }]);
  });

  it('takes a polled copy changed elsewhere, but never over unsaved typing', async () => {
    stubApi({ 'PATCH /api/todos/a': echo });
    const { result, rerender } = setup();
    rerender({ t: todo({ title: 'Buy bread' }) });
    expect(result.current.title).toBe('Buy bread');
    act(() => result.current.setBody('typing'));
    rerender({ t: todo({ title: 'Buy eggs', body: 'theirs' }) });
    expect(result.current.title).toBe('Buy bread');
    expect(result.current.body).toBe('typing');
  });

  it('a polled copy of what it just saved changes nothing', async () => {
    stubApi({ 'PATCH /api/todos/a': (c: Call) => todo({ title: 'Buy milk', body: (c.body as { body: string }).body.trim() }) });
    const { result, rerender } = setup();
    act(() => result.current.setBody('note '));
    await act(() => result.current.flush());
    // The server trimmed it; the poll brings that back, but the text being edited here stays as typed.
    rerender({ t: todo({ body: 'note' }) });
    expect(result.current.body).toBe('note ');
  });

  it('typing during a save saves again once it lands', async () => {
    let land: () => void = () => {};
    const calls = stubApi({
      'PATCH /api/todos/a': (c: Call) => (patches(calls).length === 1 ? new Promise((r) => { land = () => r(echo(c)); }) : echo(c)),
    });
    const { result } = setup();
    act(() => result.current.setBody('one'));
    let first!: Promise<void>;
    act(() => { first = result.current.flush(); });
    await waitFor(() => expect(result.current.state).toBe('saving'));
    act(() => result.current.setBody('one two'));
    await act(() => result.current.flush());
    expect(patches(calls)).toEqual([{ body: 'one' }]);
    await act(async () => {
      land();
      await first;
    });
    await waitFor(() => expect(patches(calls)).toEqual([{ body: 'one' }, { body: 'one two' }]));
    await waitFor(() => expect(result.current.state).toBe('saved'));
  });

  it('a failed save says so and keeps the text', async () => {
    stubApi({ 'PATCH /api/todos/a': new Reply(500, { code: 'internal', message: 'disk full' }) });
    render(<Toaster />);
    const { result } = setup();
    act(() => result.current.setBody('lost?'));
    await act(() => result.current.flush());
    expect(result.current.state).toBe('error');
    expect(result.current.body).toBe('lost?');
    expect(await screen.findByText(/The todo could not be saved: disk full/)).toBeInTheDocument();
    // Typing on keeps the error showing until a save works.
    act(() => result.current.setBody('lost?!'));
    expect(result.current.state).toBe('error');
  });

  it('saves what is unsaved when it goes away, and nothing once stopped (the todo is being deleted)', async () => {
    const calls = stubApi({ 'PATCH /api/todos/a': echo });
    const first = setup();
    act(() => first.result.current.setBody('on Back'));
    first.unmount();
    await waitFor(() => expect(patches(calls)).toEqual([{ body: 'on Back' }]));
    const second = setup();
    act(() => second.result.current.setBody('deleted'));
    act(() => second.result.current.stop());
    await act(() => second.result.current.flush());
    second.unmount();
    expect(patches(calls)).toEqual([{ body: 'on Back' }]);
  });
});
