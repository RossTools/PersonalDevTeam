import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { CreateTicketsCard, PdtToolCard, type PdtCall } from './PdtToolCard';
import { PdtProposal, shortText } from './PdtProposal';
import { isCardTool, pdtApprovalTitle, pdtTool, resultOf, watchEventsText } from './pdtTools';

afterEach(() => vi.unstubAllGlobals());

const done = (tool: string, input: unknown, result: unknown): PdtCall => ({
  tool: `mcp__pdt__${tool}`, input, status: 'done', output: JSON.stringify(result, null, 2),
});
const pills = (el: HTMLElement) => [...el.querySelectorAll('[data-slot=ticket-pill]')].map((p) => p.textContent);

describe('PDT tool calls', () => {
  it("knows PDT's tools by every agent's name for them, and which show as cards", () => {
    expect(pdtTool('mcp__pdt__create_ticket')).toBe('create_ticket');
    expect(pdtTool('mcp:pdt/watch')).toBe('watch');
    expect(pdtTool('mcp__github__create_issue')).toBeNull();
    expect(isCardTool('mcp__pdt__send_to_session')).toBe(true);
    expect(isCardTool('mcp__pdt__get_ticket')).toBe(false);
    expect(isCardTool('Bash')).toBe(false);
  });

  it("reads pdt-mcp's result from Claude's text and Codex's MCP result", () => {
    expect(resultOf('{"key": "ALP-3"}')).toEqual({ key: 'ALP-3' });
    expect(resultOf(JSON.stringify({ content: [{ type: 'text', text: '{"key":"ALP-4"}' }], structuredContent: { key: 'ALP-4' } }))).toEqual({ key: 'ALP-4' });
    expect(resultOf(JSON.stringify({ content: [{ type: 'text', text: '{"key":"ALP-5"}' }] }))).toEqual({ key: 'ALP-5' });
    expect(resultOf('that ticket no longer exists')).toBeNull();
    expect(resultOf(undefined)).toBeNull();
  });

  it('says what a watch tells about', () => {
    expect(watchEventsText([])).toBe('finishes, fails or needs you');
    expect(watchEventsText(['comment'])).toBe('someone comments');
  });

  it('titles the approval of the tools that ask', () => {
    expect(pdtApprovalTitle('mcp__pdt__create_ticket', {})).toBe('Create this ticket?');
    expect(pdtApprovalTitle('mcp__pdt__edit_ticket', { ticket: 'ALP-2' })).toBe('Change ALP-2?');
    expect(pdtApprovalTitle('mcp__pdt__get_ticket', {})).toBeNull();
    expect(pdtApprovalTitle('Bash', {})).toBeNull();
  });
});

describe('PDT tool cards', () => {
  it('create_ticket: the tickets made, a row each with key, title, stage and labels', () => {
    stubApi({});
    renderWithQuery(
      <CreateTicketsCard
        calls={[
          done('create_ticket', { project: 'ALP', title: 'Guest checkout', stage: 'todo', labels: ['agentic'] }, { created: true, key: 'ALP-31', stage: 'todo' }),
          done('create_ticket', { project: 'ALP', title: 'Link guest orders' }, { created: true, key: 'ALP-32', stage: 'draft' }),
        ]}
      />,
    );
    const card = screen.getByRole('region', { name: 'Created 2 tickets' });
    expect(pills(card)).toEqual(['ALP-31', 'ALP-32']);
    expect(card).toHaveTextContent('Guest checkout');
    expect(card).toHaveTextContent('To do');
    expect(card).toHaveTextContent('agentic');
    expect(card).toHaveTextContent('Draft');
  });

  it('create_ticket: a proposal, a running call and a failed one', () => {
    stubApi({});
    const { rerender } = renderWithQuery(<PdtToolCard call={done('create_ticket', { title: 'Idea' }, { key: 'ALP-9', proposal: true })} />);
    expect(screen.getByRole('region', { name: 'Proposed a ticket' })).toBeInTheDocument();
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__create_ticket', input: { title: 'Idea' }, status: 'running' }} />);
    expect(screen.getByRole('region', { name: 'Creating a ticket…' })).toBeInTheDocument();
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__create_ticket', input: { title: 'Idea' }, status: 'failed', output: 'no project ZZZ' }} />);
    expect(screen.getByRole('region', { name: "Didn't create a ticket" })).toHaveTextContent('no project ZZZ');
  });

  it('edit_ticket: from → to for each changed field, or what it asks before it ran', () => {
    stubApi({});
    const { rerender } = renderWithQuery(
      <PdtToolCard
        call={done('edit_ticket', { ticket: 'alp-2', stage: 'in_progress', description: 'New' }, {
          updated: true, key: 'ALP-2',
          changes: [{ field: 'stage', from: 'todo', to: 'in_progress' }, { field: 'labels', from: [], to: ['agentic'] }, { field: 'description' }],
        })}
      />,
    );
    const card = screen.getByRole('region', { name: 'Changed ALP-2' });
    expect(pills(card)).toEqual(['ALP-2']);
    expect(card).toHaveTextContent('StageTo doIn progress');
    expect(card).toHaveTextContent('Labelsnoneagentic');
    expect(card).toHaveTextContent('DescriptionRewritten');
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__edit_ticket', input: { ticket: 'ALP-2', priority: 'high' }, status: 'running' }} />);
    expect(screen.getByRole('region', { name: 'Changing ALP-2' })).toHaveTextContent('PriorityHigh');
  });

  it('link_tickets and unlink_tickets: "A blocked by B"', () => {
    stubApi({});
    const { rerender } = renderWithQuery(<PdtToolCard call={done('link_tickets', { ticket: 'ALP-32', kind: 'blockedBy', other: 'ALP-31' }, { linked: true })} />);
    const card = screen.getByRole('region', { name: 'Linked' });
    expect(card).toHaveTextContent('ALP-32blocked byALP-31');
    expect(pills(card)).toEqual(['ALP-32', 'ALP-31']);
    rerender(<PdtToolCard call={done('unlink_tickets', { ticket: 'ALP-1', kind: 'relates', other: 'ALP-2' }, { unlinked: true })} />);
    expect(screen.getByRole('region', { name: 'Unlinked' })).toHaveTextContent('ALP-1no longer relates toALP-2');
  });

  it('send_to_session: the quoted message, who gets it, queued', () => {
    stubApi({});
    renderWithQuery(<PdtToolCard call={done('send_to_session', { session: 's9', text: 'Skip the e2e run locally.' }, { queued: true, to: 'Coder · ALP-19', ticket: 'ALP-19' })} />);
    const card = screen.getByRole('region', { name: 'Message to Coder · ALP-19' });
    expect(within(card).getByText('Skip the e2e run locally.').tagName).toBe('BLOCKQUOTE');
    expect(card).toHaveTextContent('Queued');
  });

  it('add_ticket_comment: the comment previewed on its ticket', () => {
    stubApi({});
    renderWithQuery(<PdtToolCard call={done('add_ticket_comment', { body: 'Found a **second** bug.' }, { commented: true, key: 'ALP-4' })} />);
    const card = screen.getByRole('region', { name: 'Commented on ALP-4' });
    expect(within(card).getByText('second').tagName).toBe('STRONG');
  });

  it('workflow_run: the run with its status and a link to it', async () => {
    const calls = stubApi({ 'GET /api/workflow-runs/r1': { id: 'r1', workflowId: 'wf1', workflowName: 'Cut a release', status: 'running', error: null, steps: [] } });
    renderWithQuery(<PdtToolCard call={done('workflow_run', { name: 'Cut a release', project: 'ALP' }, { runId: 'r1', workflow: 'Cut a release' })} />);
    const card = screen.getByRole('region', { name: 'Started Cut a release' });
    expect(await within(card).findByText('Running')).toBeInTheDocument();
    expect(within(card).getByRole('link', { name: 'Open the run' })).toHaveAttribute('href', '/workflows/wf1/runs/r1');
    expect(calls.map((c) => c.path)).toContain('/api/workflow-runs/r1');
  });

  it('watch: what it watches and tells about, and Stop watching', async () => {
    const watch = { id: 'w1', target: 'ticket', targetId: 't31', targetLabel: 'ALP-31', sessionId: 'pm1', watcher: 'PM', events: ['finished', 'failed', 'needs_you'], once: true, createdAt: 1 };
    const calls = stubApi({ 'GET /api/watches': [watch], 'DELETE /api/watches/w1': undefined });
    renderWithQuery(<PdtToolCard call={done('watch', { ticket: 'ALP-31' }, { watching: [watch] })} />);
    const card = screen.getByRole('region', { name: 'Watching ALP-31' });
    expect(card).toHaveTextContent("you'll be told when it finishes, fails or needs you");
    await userEvent.click(within(card).getByRole('button', { name: 'Stop watching' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/watches/w1')).toBe(true));
    expect(await screen.findByRole('region', { name: 'Watched ALP-31' })).toHaveTextContent('No longer watching');
  });
});

describe('PDT approval previews', () => {
  it('create_ticket: title, a short description, stage, priority, labels, parent and why', () => {
    stubApi({});
    renderWithQuery(
      <PdtProposal
        tool="mcp__pdt__create_ticket"
        input={{ project: 'ALP', title: 'Guest checkout', description: 'Orders without an account.\n\n## Scope\n- lots', stage: 'todo', priority: 'high', labels: ['agentic'], parent: 'ALP-1', reason: 'Users asked.' }}
      />,
    );
    for (const text of ['Guest checkout', 'Orders without an account.', 'To do', 'High', 'agentic', 'Why: Users asked.']) {
      expect(document.body).toHaveTextContent(text);
    }
    expect(document.body).not.toHaveTextContent('Scope');
    expect(pills(document.body)).toEqual(['ALP-1']);
  });

  it('edit_ticket, link_tickets and send_to_session say what changes', async () => {
    stubApi({ 'GET /api/sessions/s9': { id: 's9', title: 'Coder · ALP-19' } });
    const { rerender } = renderWithQuery(<PdtProposal tool="mcp__pdt__edit_ticket" input={{ ticket: 'ALP-2', stage: 'ready', description: 'Shorter.' }} />);
    expect(document.body).toHaveTextContent('StageReady');
    expect(document.body).toHaveTextContent('New descriptionShorter.');
    rerender(<PdtProposal tool="mcp__pdt__link_tickets" input={{ ticket: 'ALP-2', kind: 'blocks', other: 'ALP-3' }} />);
    expect(document.body).toHaveTextContent('ALP-2blocksALP-3');
    rerender(<PdtProposal tool="mcp__pdt__send_to_session" input={{ session: 's9', text: 'Commit and submit.' }} />);
    expect(await screen.findByText('Coder · ALP-19')).toBeInTheDocument();
    expect(screen.getByText('Commit and submit.').tagName).toBe('BLOCKQUOTE');
  });

  it('cuts a description to its first paragraph', () => {
    expect(shortText('# Title\n\nMore')).toBe('Title');
    expect(shortText('a'.repeat(300))).toHaveLength(220);
  });
});
