import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import type { McpServerView } from '../api/gen/McpServerView';
import { McpServersPage, audience, pairsOf, pairsValue, signInUrlOk } from './McpServersPage';

const access = { enabled: true, pm: true, tickets: true, personas: [], user: true, projects: [], offIn: [], tools: { default: 'ask' as const, overrides: {} } };

const slack: McpServerView = {
  connection: {
    id: 'c1', name: 'Slack', kind: 'mcp', url: 'https://mcp.slack.com/mcp', config: { transport: 'http', oauth: {} },
    hasSecret: false, createdAt: 0, updatedAt: 0, lastError: null, lastErrorAt: null,
  },
  agentName: 'slack',
  access,
  transport: 'http',
  oauth: { signedIn: true, account: 'Acme', expiresAt: null, signedInAt: 0, problem: null },
  tools: null,
};

const base = {
  'GET /api/secrets': [],
  'GET /api/projects': [],
  'GET /api/personas': [],
};

describe('MCP servers', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('lists servers with who gets them and their sign-in, and tests one to set its tools', async () => {
    const calls = stubApi({
      ...base,
      'GET /api/mcp-servers': { servers: [slack], onlyPdt: false },
      'POST /api/mcp-servers/c1/test': {
        tools: { server: 'Slack', tools: [
          { name: 'search', title: null, description: 'Searches.', inputSchema: {}, outputSchema: null },
          { name: 'post_message', title: null, description: 'Posts.', inputSchema: {}, outputSchema: null },
        ], error: null, listedAt: 1, triedAt: 1 },
        resources: [],
        prompts: [{ name: 'summarize', detail: null, description: '' }],
      },
      'PATCH /api/connections/c1': slack.connection,
    });
    renderWithQuery(<McpServersPage />);
    expect(await screen.findByText('Slack')).toBeInTheDocument();
    expect(screen.getByText('For the PM, ticket agents, your sessions')).toBeInTheDocument();
    expect(screen.getByText('Signed in as Acme')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Test' }));
    const tools = await screen.findByRole('list', { name: 'Slack tools' });
    expect(within(tools).getByText('post_message')).toBeInTheDocument();
    expect(screen.getByText('Prompts: summarize')).toBeInTheDocument();
    const group = screen.getByRole('radiogroup', { name: 'post_message: allow, ask or block' });
    await userEvent.click(within(group).getByRole('radio', { name: 'Block' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PATCH')).toBe(true));
    const patch = calls.find((c) => c.method === 'PATCH')!;
    expect((patch.body as { config: { agents: typeof access } }).config.agents.tools.overrides).toEqual({ post_message: 'block' });
  });

  it('says when a server needs signing in again, and turns on Use only PDT\'s MCP servers', async () => {
    const failed = { ...slack, oauth: { ...slack.oauth!, problem: 'token revoked' } };
    const calls = stubApi({
      ...base,
      'GET /api/mcp-servers': { servers: [failed], onlyPdt: false },
      'PUT /api/host/settings': {},
    });
    renderWithQuery(<McpServersPage />);
    expect(await screen.findByText('Sign-in failed: token revoked')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign in again' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('switch', { name: "Use only PDT's MCP servers" }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ mcpOnlyPdt: true }));
  });

  it("never sends the browser to a sign-in address that isn't a web address", async () => {
    stubApi({
      ...base,
      'GET /api/mcp-servers': { servers: [slack], onlyPdt: false },
      'POST /api/mcp-servers/c1/sign-in': { url: "javascript:fetch('/api/sessions')//" },
    });
    const tab = { opener: {}, location: { href: 'about:blank' }, close: vi.fn() };
    vi.stubGlobal('open', vi.fn(() => tab));
    renderWithQuery(<McpServersPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign in again' }));
    expect(await screen.findByText(/isn't a web address/)).toBeInTheDocument();
    expect(tab.close).toHaveBeenCalled();
    expect(tab.location.href).toBe('about:blank');
    expect(['https://a.example/authorize?x=1', 'http://127.0.0.1:9/authorize'].every(signInUrlOk)).toBe(true);
    expect(['javascript:alert(1)', 'data:text/html,x', 'JavaScript:alert(1)', 'not a url'].some(signInUrlOk)).toBe(false);
  });

  it('keeps secrets as references, and sums up who gets a server', () => {
    const pairs = pairsOf({ A: 'x', B: { $secret: 'KEY' }, C: 'secret:OTHER' });
    expect(pairs).toEqual([
      { key: 'A', secret: false, value: 'x' },
      { key: 'B', secret: true, value: 'KEY' },
      { key: 'C', secret: true, value: 'OTHER' },
    ]);
    expect(pairsValue(pairs)).toEqual({ A: 'x', B: { $secret: 'KEY' }, C: { $secret: 'OTHER' } });
    expect(audience({ ...access, enabled: false })).toBe('Workflows only');
    expect(audience({ ...access, pm: false, user: false, projects: ['p'] })).toBe('For ticket agents in 1 project');
  });
});
