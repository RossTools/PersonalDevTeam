import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { ConversationState } from '../api/gen/ConversationState';
import type { Session } from '../api/gen/Session';
import { SessionView } from '../sessions/SessionView';
import { Reply, renderWithQuery, stubApi } from '../test-utils';
import { ConversationComposer, type ConversationComposerProps } from './ConversationComposer';
import { ConversationView } from './ConversationView';
import { FakeSocket } from './fakeSocket';
import { MessageList } from './MessageList';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

const ev = (seq: number, event: AgentEvent, extra: Partial<ConversationEvent> = {}): ConversationEvent =>
  ({ seq, pos: seq, ts: 1_000_000 + seq * 1000, event, itemId: `i${seq}`, turnId: 'u1', ...extra });

const session: Session = {
  id: 's1', projectId: 'p', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PDT', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null, activity: 'idle',
  attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation', model: null, permissionMode: 'ask',
};
const idle: ConversationState = { live: true, turnRunning: false, permissionMode: 'ask', pending: [], terminalOpen: false };

afterEach(() => {
  FakeSocket.all = [];
  localStorage.clear();
  vi.unstubAllGlobals();
});

describe('MessageList', () => {
  const list = (rows: ConversationEvent[], extra: Partial<Parameters<typeof MessageList>[0]> = {}) =>
    render(<MessageList rows={rows} provisional={[]} streams={{}} turnRunning={false} pending={[]} onAnswer={vi.fn(async () => {})} {...extra} />);

  it('renders every kind of event', async () => {
    list([
      ev(1, { kind: 'user_message', text: 'Fix the bug', images: [], source: 'pdt' }),
      ev(2, { kind: 'turn', phase: 'started' }),
      ev(3, { kind: 'reasoning', summary: 'Looking at it' }),
      ev(4, { kind: 'agent_message', id: 'a', markdown: 'Let me check', phase: 'commentary' }),
      ev(5, { kind: 'tool_call', id: 't', tool: 'Bash', title: 'cargo test', input: { command: 'cargo test' }, status: 'failed', exitCode: 101, output: 'boom' }),
      ev(6, { kind: 'agent_message', id: 'b', markdown: 'It **works** now', phase: 'final' }),
      ev(7, { kind: 'request', id: 'r1', requestKind: 'approval', tool: 'Edit', title: 'a.ts', input: {}, canAlways: false, state: 'answered', answer: { type: 'deny', message: 'no' } }),
      ev(8, { kind: 'turn', phase: 'ended', status: 'interrupted', durationMs: 12_000, reason: 'Stopped by you' }),
      ev(9, { kind: 'usage', contextUsed: 1, contextWindow: 2, rateLimits: [] }),
      ev(10, { kind: 'notice', level: 'warning', text: 'Rate limited' }),
      ev(11, { kind: 'error', text: 'Something broke', retryable: false }),
      ev(12, { kind: 'compaction' }),
      ev(13, { kind: 'plan', markdown: '1. First' }),
      ev(14, { kind: 'turn_diff', diff: '+x' }),
      ev(15, { kind: 'unknown', raw: { weird: true } }),
    ], { empty: <p>Nothing yet</p> });
    expect(screen.getByText('Fix the bug')).toBeInTheDocument();
    expect(screen.getByText('Thinking')).toBeInTheDocument();
    expect(screen.getByText('Let me check')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Run cargo test exit 101/ })).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Failed' })).toBeInTheDocument();
    expect(screen.getByText('works').tagName).toBe('STRONG');
    expect(screen.getByText('Denied: no')).toBeInTheDocument();
    expect(screen.getByTestId('turn-footer')).toHaveTextContent('Interrupted after 12 s · Stopped by you');
    expect(screen.getByText('Rate limited')).toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent('Something broke');
    expect(screen.getByText('Context compacted')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Plan' })).toHaveTextContent('First');
    expect(screen.getByText('Unrecognised event')).toBeInTheDocument();
    expect(screen.queryByText('Nothing yet')).not.toBeInTheDocument();
    expect(screen.queryByText('+x')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Run cargo test/ }));
    expect(screen.getByText('boom')).toBeInTheDocument();
  });

  it('shows an edit’s diff, the turn’s changes and Continue after a cut-off turn', async () => {
    const diff = '--- a/a.ts\n+++ b/a.ts\n@@ -1,2 +1,3 @@\n keep\n-old\n+new\n+more\n';
    const onContinue = vi.fn(async () => {});
    list([
      ev(1, { kind: 'tool_call', id: 'e', tool: 'Edit', title: 'a.ts', input: { file_path: '/r/a.ts' }, status: 'done', output: 'updated', diff }),
      ev(2, { kind: 'turn_diff', diff }),
      ev(3, { kind: 'turn', phase: 'ended', status: 'interrupted', reason: 'PDT restarted' }),
    ], { onContinue });
    const row = screen.getByRole('button', { name: /Edit a\.ts \+2.?−1/ });
    await userEvent.click(row);
    expect(screen.getByText('more').closest('tr')).toHaveAttribute('data-kind', 'add');
    expect(screen.queryByText('updated')).not.toBeInTheDocument();
    expect(screen.queryByText('Input')).not.toBeInTheDocument();

    const footer = screen.getByTestId('turn-footer');
    await userEvent.click(within(footer).getByRole('button', { name: /Changes/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Changes in this turn' });
    expect(within(dialog).getByRole('region', { name: 'a.ts' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    await userEvent.click(within(footer).getByRole('button', { name: 'Continue' }));
    expect(onContinue).toHaveBeenCalledTimes(1);
  });

  it('offers Continue only on the last turn, and not once something follows it', () => {
    const cut = (seq: number) => ev(seq, { kind: 'turn', phase: 'ended', status: 'interrupted' }, { itemId: `end${seq}` });
    const { unmount } = list([cut(1), ev(2, { kind: 'user_message', text: 'next', images: [], source: 'pdt' })], { onContinue: vi.fn(async () => {}) });
    expect(screen.queryByRole('button', { name: 'Continue' })).not.toBeInTheDocument();
    unmount();
    list([cut(1)], { onContinue: vi.fn(async () => {}), pending: [{ id: 'q', text: 'x', delivery: 'next', images: [] }] });
    expect(screen.queryByRole('button', { name: 'Continue' })).not.toBeInTheDocument();
  });

  it('edit approvals show their diff; Claude plans can go straight to Accept edits', async () => {
    const onAnswer = vi.fn(async () => {});
    list([
      ev(1, { kind: 'request', id: 'r1', requestKind: 'approval', tool: 'Edit', title: 'a.ts', input: {}, canAlways: false, state: 'open', diff: '--- a/a.ts\n+++ b/a.ts\n@@ @@\n-x\n+y\n' }),
      ev(2, { kind: 'request', id: 'p1', requestKind: 'plan', tool: 'ExitPlanMode', title: '', input: { plan: '1. Do it' }, canAlways: false, state: 'open' }),
    ], { onAnswer });
    const card = screen.getByRole('region', { name: 'Edit this file?' });
    expect(within(card).getByText('y').closest('tr')).toHaveAttribute('data-kind', 'add');
    await userEvent.click(screen.getByRole('button', { name: 'Approve, accept edits' }));
    expect(onAnswer).toHaveBeenCalledWith('p1', { type: 'plan', approve: true, mode: 'accept_edits' });
  });

  it('Codex plans have no Accept edits choice', () => {
    list([ev(1, { kind: 'request', id: 'p1', requestKind: 'plan', tool: 'plan', title: '', input: { plan: 'x' }, canAlways: false, state: 'open' })], { agent: 'codex' });
    expect(screen.queryByRole('button', { name: 'Approve, accept edits' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Approve plan/ })).toBeInTheDocument();
  });

  it('shows the images sent with a message', () => {
    const aid = '01a1546a-24ca-72e6-8206-3ae44a03d42d';
    list(
      [ev(1, { kind: 'user_message', text: '', images: [aid, 'image/png'], source: 'pdt' })],
      { imageUrl: (id) => `/att/${id}` },
    );
    expect(screen.getByRole('img', { name: 'Image 1' })).toHaveAttribute('src', `/att/${aid}`);
    expect(screen.getByRole('img', { name: 'Image 2' }).tagName).toBe('SPAN');
  });

  it('collapses a finished turn’s tool calls into a summary', async () => {
    list([
      ev(1, { kind: 'tool_call', id: 'a', tool: 'Bash', title: 'ls', input: {}, status: 'done' }),
      ev(2, { kind: 'tool_call', id: 'b', tool: 'Edit', title: 'a.ts', input: {}, status: 'done' }),
      ev(3, { kind: 'turn', phase: 'ended', status: 'completed', durationMs: 500 }),
    ]);
    const summary = screen.getByRole('button', { name: /Ran 1 command, edited 1 file/ });
    expect(screen.queryByText('ls')).not.toBeInTheDocument();
    await userEvent.click(summary);
    expect(screen.getByText('ls')).toBeInTheDocument();
    expect(screen.getByTestId('turn-footer')).toHaveTextContent('Done in 500 ms');
  });

  it('shows the running tool while a turn works, streaming text and queued messages', () => {
    list(
      [
        ev(1, { kind: 'turn', phase: 'started' }),
        ev(2, { kind: 'tool_call', id: 'a', tool: 'Bash', title: 'cargo build', input: {}, status: 'running' }),
      ],
      {
        turnRunning: true,
        provisional: [{ itemId: 'm:1', markdown: 'Streaming…' }],
        pending: [{ id: 'q', text: 'also do X', delivery: 'next', images: [] }],
      },
    );
    expect(screen.getByTestId('working')).toHaveTextContent(/Running\s*cargo build/);
    expect(screen.getByText('Streaming…')).toBeInTheDocument();
    expect(screen.getByText('Queued')).toBeInTheDocument();
    expect(screen.getByText('also do X')).toBeInTheDocument();
  });

  it('says where a turn or an answer came from when it was the terminal', () => {
    list([
      ev(1, { kind: 'user_message', text: 'typed in the TUI', images: [], source: 'terminal' }),
      ev(2, { kind: 'request', id: 'r1', requestKind: 'approval', tool: 'Bash', title: 'ls', input: {}, canAlways: false, state: 'answered' }),
      ev(3, { kind: 'request', id: 'r2', requestKind: 'approval', tool: 'Bash', title: 'pwd', input: {}, canAlways: false, state: 'expired' }),
    ]);
    expect(screen.getByText('From the terminal')).toBeInTheDocument();
    expect(screen.getByText('Answered in the terminal')).toBeInTheDocument();
    expect(screen.getByText('Expired')).toBeInTheDocument();
  });

  it('an open request is an approval card; y answers the newest one', async () => {
    const onAnswer = vi.fn(async () => {});
    list([
      ev(1, { kind: 'request', id: 'r1', requestKind: 'approval', tool: 'Bash', title: 'rm a', input: {}, canAlways: true, state: 'open' }),
      ev(2, { kind: 'request', id: 'r2', requestKind: 'approval', tool: 'Bash', title: 'rm b', input: {}, canAlways: true, state: 'open' }),
    ], { onAnswer });
    expect(screen.getAllByRole('region', { name: 'Run this command?' })).toHaveLength(2);
    await userEvent.keyboard('y');
    expect(onAnswer).toHaveBeenCalledTimes(1);
    expect(onAnswer).toHaveBeenCalledWith('r2', { type: 'approve', always: false });
  });
});

describe('ConversationComposer', () => {
  const setup = (over: Partial<ConversationComposerProps> = {}) => {
    const props: ConversationComposerProps = {
      turnRunning: false, onSend: vi.fn(async () => {}), onInterrupt: vi.fn(), models: [{ value: 'opus', displayName: 'Opus' }],
      model: null, onModel: vi.fn(), permissionModes: [], permissionMode: 'ask', onPermissionMode: vi.fn(), ...over,
    };
    render(<ConversationComposer {...props} />);
    return props;
  };

  it('/ lists commands; arrows and Enter pick one; TUI-only ones are shown but not picked', async () => {
    const p = setup({
      commands: [{ name: 'compact', description: 'Compact the conversation' }, { name: 'context', description: 'Show context' }],
      terminalCommands: ['config'],
      turnRunning: true,
    });
    const box = screen.getByRole('textbox', { name: 'Message' });
    await userEvent.type(box, '/co');
    const list = screen.getByRole('listbox', { name: 'Commands' });
    expect(within(list).getAllByRole('option').map((o) => o.textContent)).toEqual([
      '/compactCompact the conversation',
      '/contextShow context',
      '/configOnly in the terminal',
    ]);
    expect(box).toHaveAttribute('aria-activedescendant', within(list).getAllByRole('option')[0].id);
    await userEvent.keyboard('{ArrowDown}{ArrowDown}{Enter}');
    expect(box).toHaveValue('/co');
    expect(p.onSend).not.toHaveBeenCalled();
    await userEvent.keyboard('{ArrowUp}{Enter}');
    expect(box).toHaveValue('/context ');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    // Escape closes the list and does not stop the agent.
    await userEvent.clear(box);
    await userEvent.type(box, '/');
    expect(screen.getByRole('listbox', { name: 'Commands' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    expect(p.onInterrupt).not.toHaveBeenCalled();
  });

  it('@ suggests files from the session folder and inserts the path', async () => {
    const onSearchFiles = vi.fn(async (q: string) => (q === 'ma' ? ['src/main.rs', 'docs/manual.md'] : []));
    setup({ onSearchFiles });
    const box = screen.getByRole('textbox', { name: 'Message' });
    await userEvent.type(box, 'read @ma');
    const list = await screen.findByRole('listbox', { name: 'Files' });
    expect(within(list).getAllByRole('option').map((o) => o.textContent)).toEqual(['src/main.rs', 'docs/manual.md']);
    await userEvent.click(within(list).getByRole('option', { name: 'docs/manual.md' }));
    expect(box).toHaveValue('read @docs/manual.md ');
    expect(onSearchFiles).toHaveBeenLastCalledWith('ma');
  });

  it('pasted images upload, show as thumbnails and go with the message (even without text)', async () => {
    const createObjectURL = vi.fn(() => 'blob:thumb');
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL, revokeObjectURL: vi.fn() }));
    let finish!: (v: { id: string }) => void;
    const onUpload = vi.fn(() => new Promise<{ id: string }>((r) => { finish = r; }));
    const p = setup({ onUpload });
    const box = screen.getByRole('textbox', { name: 'Message' });
    const png = new File(['x'], 'shot.png', { type: 'image/png' });
    await userEvent.click(box);
    await userEvent.paste({ files: [png] } as unknown as string);
    expect(onUpload).toHaveBeenCalledWith(png);
    expect(screen.getByRole('img', { name: 'shot.png' })).toHaveAttribute('src', 'blob:thumb');
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await act(async () => finish({ id: 'att-1' }));
    await userEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(p.onSend).toHaveBeenCalledWith('', undefined, ['att-1']);
    expect(screen.queryByRole('img', { name: 'shot.png' })).not.toBeInTheDocument();
    // Not an image: refused before uploading.
    await userEvent.click(box);
    await userEvent.paste({ files: [new File(['x'], 'a.svg', { type: 'image/svg+xml' })] } as unknown as string);
    expect(screen.getByText('Only PNG, JPEG, GIF and WebP images can be attached')).toBeInTheDocument();
    expect(onUpload).toHaveBeenCalledTimes(1);
  });

  it('warns near a rate limit and says when one is hit', () => {
    setup({ limit: { text: 'Rate limited until 3:40 PM (5-hour limit)', reached: true } });
    expect(screen.getByRole('status')).toHaveTextContent('Rate limited until 3:40 PM (5-hour limit). Nothing is sent until you send again.');
  });

  it('Enter sends; Shift+Enter adds a line', async () => {
    const p = setup();
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'one{Shift>}{Enter}{/Shift}two{Enter}');
    expect(p.onSend).toHaveBeenCalledWith('one\ntwo', undefined);
  });

  it('while working: Enter queues, Cmd/Ctrl+Enter sends now, Escape and Stop interrupt', async () => {
    const p = setup({ turnRunning: true });
    const box = screen.getByRole('textbox', { name: 'Message' });
    await userEvent.type(box, 'later{Enter}');
    expect(p.onSend).toHaveBeenLastCalledWith('later', 'next');
    await userEvent.type(box, 'now{Control>}{Enter}{/Control}');
    expect(p.onSend).toHaveBeenLastCalledWith('now', 'now');
    await userEvent.type(box, '{Escape}');
    expect(p.onInterrupt).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole('button', { name: /Stop/ }));
    expect(p.onInterrupt).toHaveBeenCalledTimes(2);
  });

  it('Escape does nothing when idle; shows why the agent stopped', async () => {
    const p = setup({ stopped: 'exit 1: out of memory' });
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), '{Escape}');
    expect(p.onInterrupt).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent('Claude stopped');
    expect(screen.getByRole('alert')).toHaveTextContent('Send a message to start it again');
  });

  it('picks a model ("Default" is null) and a permission mode', async () => {
    const p = setup();
    await userEvent.click(screen.getByRole('button', { name: /Model/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Opus' }));
    expect(p.onModel).toHaveBeenCalledWith('opus');
    await userEvent.click(screen.getByRole('button', { name: /Permission mode/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Bypass (dangerous)' }));
    expect(p.onPermissionMode).toHaveBeenCalledWith('bypass');
  });
});

describe('ConversationView', () => {
  const caps = { conversation: true, permissionModes: ['ask', 'accept_edits', 'plan', 'bypass'], models: [], commands: [] };
  const server = () => FakeSocket.all[FakeSocket.all.length - 1];

  it('replays, then follows live events and sends messages', async () => {
    const calls = stubApi({
      'GET /api/agents/claude/capabilities': caps,
      'POST /api/sessions/s1/messages': { id: 'q', text: 'hello', delivery: 'next', images: [] },
    });
    renderWithQuery(<ConversationView session={session} socketFactory={(u) => new FakeSocket(u)} />);
    expect(server().url).toMatch(/\/api\/sessions\/s1\/events\?after=0$/);
    act(() => {
      server().open();
      server().send({ type: 'event', event: ev(1, { kind: 'user_message', text: 'first', images: [], source: 'pdt' }) });
      server().send({ type: 'state', state: idle });
      server().send({ type: 'ready' });
    });
    expect(screen.getByText('first')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'hello{Enter}');
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s1/messages')?.body).toEqual({ text: 'hello' }));
    act(() => {
      server().send({ type: 'state', state: { ...idle, turnRunning: true, pending: [{ id: 'q', text: 'hello', delivery: 'next', images: [] }] } });
    });
    expect(screen.getByText('Queued')).toBeInTheDocument();
    act(() => {
      server().send({ type: 'delta', itemId: 'm:1', field: 'markdown', text: 'Hi the' });
      server().send({ type: 'delta', itemId: 'm:1', field: 'markdown', text: 're' });
    });
    expect(screen.getByText('Hi there')).toBeInTheDocument();
    // Reconnecting asks for what came after the last seq.
    act(() => server().close());
    await waitFor(() => expect(FakeSocket.all).toHaveLength(2), { timeout: 2000 });
    expect(server().url).toMatch(/after=1$/);
  });

  it('answers requests, and says so when another device got there first', async () => {
    const calls = stubApi({
      'GET /api/agents/claude/capabilities': caps,
      'POST /api/sessions/s1/requests/r1': new Reply(409, { code: 'request_closed', message: 'closed' }),
    });
    renderWithQuery(<ConversationView session={session} socketFactory={(u) => new FakeSocket(u)} />);
    act(() => {
      server().open();
      server().send({ type: 'event', event: ev(1, { kind: 'request', id: 'r1', requestKind: 'approval', tool: 'Bash', title: 'ls', input: {}, canAlways: false, state: 'open' }) });
      server().send({ type: 'state', state: idle });
      server().send({ type: 'ready' });
    });
    await userEvent.click(screen.getByRole('button', { name: 'Approve' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s1/requests/r1')?.body).toEqual({ answer: { type: 'approve', always: false } }));
    expect(await screen.findByText('That request was already answered or has expired.')).toBeInTheDocument();
  });

  it('interrupts and changes the permission mode', async () => {
    const calls = stubApi({
      'GET /api/agents/claude/capabilities': caps,
      'POST /api/sessions/s1/interrupt': undefined,
      'PATCH /api/sessions/s1': { ...session, permissionMode: 'plan' },
    });
    renderWithQuery(<ConversationView session={session} socketFactory={(u) => new FakeSocket(u)} />);
    act(() => {
      server().open();
      server().send({ type: 'state', state: { ...idle, turnRunning: true } });
      server().send({ type: 'ready' });
    });
    await userEvent.click(screen.getByRole('button', { name: /Stop/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions/s1/interrupt')).toBe(true));
    await userEvent.click(screen.getByRole('button', { name: /Permission mode/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Plan' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ permissionMode: 'plan' }));
  });
});

describe('SessionView for a conversation', () => {
  it('shows the conversation instead of a terminal, without Compose or Restart', async () => {
    vi.stubGlobal('WebSocket', FakeSocket);
    stubApi({
      'GET /api/agents/claude/capabilities': { conversation: true, permissionModes: [], models: [], commands: [] },
      'GET /api/projects': [],
      'POST /api/sessions/s1/view': undefined,
    });
    renderWithQuery(<SessionView session={{ ...session, status: 'finished' }} onRemoved={() => {}} />);
    expect(screen.getByRole('log', { name: 'Conversation' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Message' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Restart' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Session actions' }));
    const menu = await screen.findByRole('menu');
    expect(within(menu).queryByRole('menuitem', { name: 'Compose' })).not.toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: 'Restart' })).not.toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: /Remove/ })).toBeInTheDocument();
  });
});

describe('SessionView for a Codex conversation', () => {
  const codex: Session = { ...session, agent: 'codex' };
  const caps = { conversation: true, permissionModes: [], models: [], commands: [] };
  const server = () => FakeSocket.all[FakeSocket.all.length - 1];
  const routes = {
    'GET /api/agents/codex/capabilities': caps,
    'GET /api/projects': [],
    'POST /api/sessions/s1/view': undefined,
    'POST /api/sessions/s1/terminal': undefined,
    'DELETE /api/sessions/s1/terminal': undefined,
  };
  const show = (rows: ConversationEvent[], state: ConversationState = idle) => {
    renderWithQuery(<SessionView session={codex} onRemoved={() => {}} socketFactory={(u) => new FakeSocket(u)} />);
    act(() => {
      server().open();
      for (const r of rows) server().send({ type: 'event', event: r });
      server().send({ type: 'state', state });
      server().send({ type: 'ready' });
    });
  };
  const turn = [ev(1, { kind: 'user_message', text: 'hi', images: [], source: 'pdt' }), ev(2, { kind: 'turn', phase: 'started' })];

  it('cannot open the terminal before the first turn', () => {
    stubApi(routes);
    show([]);
    expect(screen.getByRole('radio', { name: 'Conversation' })).toBeChecked();
    const terminal = screen.getByRole('radio', { name: 'Terminal' });
    expect(terminal).toBeDisabled();
    expect(terminal.closest('[title]')).toHaveAttribute('title', 'Send a first message, then open the terminal');
  });

  it('opens the TUI, goes back to the conversation, and closes it from the menu', async () => {
    const calls = stubApi(routes);
    show(turn);
    expect(FakeSocket.all).toHaveLength(1);
    await userEvent.click(screen.getByRole('radio', { name: 'Terminal' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions/s1/terminal')?.body).toEqual({ cols: 120, rows: 32 }));
    expect(await screen.findByTestId('terminal-view')).toHaveTextContent('s1');
    act(() => server().send({ type: 'state', state: { ...idle, terminalOpen: true } }));
    expect(screen.getByRole('radio', { name: 'Terminal' })).toBeChecked();

    // Back to the conversation: the TUI keeps running, and says so.
    await userEvent.click(screen.getByRole('radio', { name: 'Conversation' }));
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
    expect(screen.getByRole('log', { name: 'Conversation' })).toBeInTheDocument();
    expect(screen.getByText('Also open in the terminal')).toBeInTheDocument();
    // Showing it again doesn't open a second one.
    await userEvent.click(screen.getByRole('radio', { name: 'Terminal' }));
    expect(await screen.findByTestId('terminal-view')).toBeInTheDocument();
    expect(calls.filter((c) => c.method === 'POST' && c.path === '/api/sessions/s1/terminal')).toHaveLength(1);

    await userEvent.click(screen.getByRole('button', { name: 'Session actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Close terminal' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/sessions/s1/terminal')).toBe(true));
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
    // One events socket for the header and the views.
    expect(FakeSocket.all).toHaveLength(1);
  });

  it('goes back to the conversation when the TUI exits by itself', async () => {
    stubApi(routes);
    show(turn, { ...idle, terminalOpen: true });
    await userEvent.click(screen.getByRole('radio', { name: 'Terminal' }));
    expect(await screen.findByTestId('terminal-view')).toBeInTheDocument();
    act(() => server().send({ type: 'state', state: idle }));
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Conversation' })).toBeChecked();
    expect(screen.queryByText('Also open in the terminal')).not.toBeInTheDocument();
  });

  it('explains a refused open', async () => {
    stubApi({ ...routes, 'POST /api/sessions/s1/terminal': new Reply(409, { code: 'no_turn_yet', message: 'no turn' }) });
    show(turn);
    await userEvent.click(screen.getByRole('radio', { name: 'Terminal' }));
    expect(await screen.findByText('Send a first message, then open the terminal.')).toBeInTheDocument();
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
  });

});

describe('SessionView for Claude', () => {
  const caps = { conversation: true, permissionModes: [], models: [], commands: [] };
  const base = { 'GET /api/agents/claude/capabilities': caps, 'GET /api/projects': [], 'POST /api/sessions/s1/view': undefined };

  it('hands a conversation to the terminal, asking before it interrupts a turn', async () => {
    let tries = 0;
    const calls = stubApi({
      ...base,
      'POST /api/sessions/s1/mode': () => (tries++ === 0 ? new Reply(409, { code: 'turn_running', message: 'working' }) : { ...session, mode: 'terminal' }),
    });
    renderWithQuery(<SessionView session={session} onRemoved={() => {}} socketFactory={(u) => new FakeSocket(u)} />);
    expect(screen.getByRole('radio', { name: 'Conversation' })).toBeChecked();
    await userEvent.click(screen.getByRole('radio', { name: 'Terminal' }));
    const ask = await screen.findByRole('alertdialog', { name: 'Interrupt and open the terminal?' });
    await userEvent.click(within(ask).getByRole('button', { name: 'Interrupt and open' }));
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/sessions/s1/mode').map((c) => c.body)).toEqual([
      { mode: 'terminal', cols: 120, rows: 32 },
      { mode: 'terminal', cols: 120, rows: 32, interrupt: true },
    ]));
  });

  it('a Claude terminal goes back to being a conversation', async () => {
    const calls = stubApi({ ...base, 'POST /api/sessions/s1/mode': { ...session } });
    renderWithQuery(<SessionView session={{ ...session, mode: 'terminal' }} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Terminal' })).toBeChecked();
    await userEvent.click(screen.getByRole('radio', { name: 'Conversation' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s1/mode')?.body).toEqual({ mode: 'conversation', cols: 120, rows: 32 }));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });
});
