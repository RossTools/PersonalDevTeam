import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { ToolCallRow } from '@ui';
import { MarkdownRefsContext } from '../common/markdownRefs';
import { CreateTicketsCard, PdtToolCard, TicketPills, type PdtCall } from './PdtToolCard';
import { PdtProposal, shortText } from './PdtProposal';
import { documentOf, editsText, filedIssue, isCardTool, issueKeyOf, issueLink, noteOf, pdtApprovalTitle, pdtTool, resultOf, stoppedTicket, watchEventsText } from './pdtTools';

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
    expect(watchEventsText([])).toBe('it finishes, fails or needs you');
    expect(watchEventsText(['comment'])).toBe('someone comments');
    expect(watchEventsText(['outcome', 'finished', 'comment'])).toBe('it finishes, an agent reports or someone comments');
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
    expect(card).toHaveTextContent('StageTo dotoIn progress');
    expect(card).toHaveTextContent('Labelsnonetoagentic');
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
    const watch = { id: 'w1', target: 'ticket', targetId: 't31', targetLabel: 'ALP-31', targetTitle: null, targetState: null, workflowId: null, sessionId: 'pm1', watcher: 'PM', events: ['finished', 'failed', 'needs_you'], once: true, createdAt: 1 };
    const calls = stubApi({ 'GET /api/watches': [watch], 'DELETE /api/watches/w1': undefined });
    renderWithQuery(<PdtToolCard call={done('watch', { ticket: 'ALP-31' }, { watching: [watch] })} />);
    const card = screen.getByRole('region', { name: 'Watching ALP-31' });
    expect(card).toHaveTextContent("you'll be told when it finishes, fails or needs you");
    await userEvent.click(within(card).getByRole('button', { name: 'Stop watching' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/watches/w1')).toBe(true));
    expect(await screen.findByRole('region', { name: 'Watched ALP-31' })).toHaveTextContent('No longer watching');
  });
});

describe('PDT tool cards: stopping work, issues, documents and notes', () => {
  const codex = (tool: string, input: unknown, result: unknown): PdtCall => ({
    tool: `mcp:pdt/${tool}`, input, status: 'done',
    output: JSON.stringify({ content: [{ type: 'text', text: JSON.stringify(result) }], structuredContent: result }),
  });

  it('reads each tool\'s input and result', () => {
    expect(stoppedTicket({ ticket: 'alp-2' }, JSON.stringify({ stopped: true, key: 'ALP-2', stage: 'todo', runsCancelled: 1, sessionsStopped: 2, worktreeRemoved: false, branchNote: null })))
      .toEqual({ key: 'ALP-2', stage: 'todo', runsCancelled: 1, sessionsStopped: 2, worktreeRemoved: false, branchNote: null });
    expect(stoppedTicket({ ticket: 'ALP-2', stage: 'ready' })).toMatchObject({ key: 'ALP-2', stage: 'ready', runsCancelled: 0 });

    expect(issueKeyOf('https://github.com/acme/shop/issues/12')).toBe('acme/shop#12');
    expect(issueKeyOf('https://example.com/x')).toBeNull();
    expect(filedIssue({ ticket: 'ALP-2' }, JSON.stringify({ repo: 'acme/shop', number: 12, issue: 'acme/shop#12', url: 'https://github.com/acme/shop/issues/12', title: 'Fix CI', ticketKey: 'ALP-2' })))
      .toEqual({ title: 'Fix CI', issue: 'acme/shop#12', url: 'https://github.com/acme/shop/issues/12', ticket: 'ALP-2' });
    // report_bug: an issue on PDT's repository, or a local ticket.
    expect(filedIssue({ title: 'Crash' }, JSON.stringify({ kind: 'issue', number: 7, url: 'https://github.com/o/pdt/issues/7' })))
      .toEqual({ title: 'Crash', issue: 'o/pdt#7', url: 'https://github.com/o/pdt/issues/7', ticket: null });
    expect(filedIssue({ title: 'Crash' }, JSON.stringify({ kind: 'ticket', key: 'PDT-9', id: 't', projectId: 'p' })))
      .toEqual({ title: 'Crash', issue: null, url: null, ticket: 'PDT-9' });

    expect(issueLink({ ticket: 'alp-1', issue: 'https://github.com/o/r/issues/1' }, JSON.stringify({ linked: true, issue: { key: 'o/r#1', url: 'https://github.com/o/r/issues/1', title: 'Crash' }, ticket: { key: 'ALP-1' } })))
      .toEqual({ ticket: 'ALP-1', key: 'o/r#1', url: 'https://github.com/o/r/issues/1', title: 'Crash' });
    expect(issueLink({ ticket: 'ALP-1', issue: 'https://github.com/o/r/issues/1' })).toEqual({ ticket: 'ALP-1', key: 'o/r#1', url: 'https://github.com/o/r/issues/1', title: null });
    expect(issueLink({ ticket: 'ALP-1', issue: 'JIRA-4' })).toMatchObject({ key: 'JIRA-4', url: null });

    expect(editsText([{ op: 'replace_section', heading: 'Scope', markdown: 'x' }])).toBe('Replaced the section \u201cScope\u201d');
    expect(editsText([{ op: 'append_text' }, { op: 'table_append_row', heading: 'Notes' }])).toBe('Added text at the end and added a table row under \u201cNotes\u201d');
    expect(editsText([{ op: 'add_sheet', name: 'Q3' }, { op: 'set_cells', sheet: 'Q3', cell: 'A1' }, { op: 'add_chart' }, { op: 'freeze' }]))
      .toBe('Added the sheet \u201cQ3\u201d, set cells from A1 in Q3 and 2 more');
    expect(editsText([])).toBeNull();
    const brief = { id: 'd1', title: 'Spec', kind: 'doc', url: null, projects: ['ALP'], tickets: ['ALP-3'] };
    expect(documentOf('mcp__pdt__document_create', { title: 'Spec', kind: 'doc', ticket: 'ALP-3' }, JSON.stringify({ ...brief, openBeside: true })))
      .toMatchObject({ id: 'd1', title: 'Spec', kind: 'doc', tickets: ['ALP-3'], openedBeside: true, change: null });
    expect(documentOf('mcp__pdt__document_create', { title: 'Plan', kind: 'sheet', ticket: 'ALP-3' }))
      .toMatchObject({ id: null, title: 'Plan', kind: 'sheet', tickets: ['ALP-3'] });
    expect(documentOf('mcp__pdt__sheet_edit', { document: 'Budget', edits: [{ op: 'add_chart' }] }))
      .toMatchObject({ id: 'Budget', title: 'Budget', kind: 'sheet', change: 'Added a chart' });
    expect(documentOf('mcp__pdt__document_link', { document: 'd1', ticket: 'ALP-4', unlink: true }, JSON.stringify(brief)))
      .toMatchObject({ title: 'Spec', change: 'No longer linked to ALP-4' });

    expect(noteOf({ title: 'Deploys', body: 'In the vault.', project: 'ALP', pinned: true }, JSON.stringify({ saved: true, id: 'n1', title: 'Deploys', project: 'ALP', pinned: true })))
      .toEqual({ id: 'n1', title: 'Deploys', body: 'In the vault.', project: 'ALP', pinned: true, replaced: false });
    expect(noteOf({ id: 'n1' }, JSON.stringify({ deleted: true, title: 'Deploys', project: null, pinned: false })))
      .toMatchObject({ id: 'n1', title: 'Deploys', project: null, pinned: false, replaced: true });
  });

  it('stop_ticket: the ticket, where it went and what stopped; running and failed', () => {
    stubApi({});
    const { rerender } = renderWithQuery(
      <PdtToolCard call={done('stop_ticket', { ticket: 'ALP-2' }, { stopped: true, key: 'ALP-2', stage: 'todo', runsCancelled: 1, sessionsStopped: 2, worktreeRemoved: true, branchNote: null })} />,
    );
    const card = screen.getByRole('region', { name: 'Stopped work on ALP-2' });
    expect(pills(card)).toEqual(['ALP-2']);
    expect(card).toHaveTextContent('To do');
    expect(card).toHaveTextContent('Cancelled 1 workflow run, stopped 2 sessions, removed its worktree');
    rerender(<PdtToolCard call={{ tool: 'mcp:pdt/stop_ticket', input: { ticket: 'ALP-2' }, status: 'running' }} />);
    expect(screen.getByRole('region', { name: 'Stopping work on ALP-2' })).toBeInTheDocument();
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__stop_ticket', input: { ticket: 'ALP-2' }, status: 'failed', output: 'no ticket ALP-2' }} />);
    expect(screen.getByRole('region', { name: "Didn't stop work on ALP-2" })).toHaveTextContent('no ticket ALP-2');
  });

  it('issue_create and report_bug: the issue with its key, link and ticket', () => {
    stubApi({});
    const { rerender } = renderWithQuery(
      <PdtToolCard call={codex('issue_create', { ticket: 'ALP-2' }, { repo: 'acme/shop', number: 12, issue: 'acme/shop#12', url: 'https://github.com/acme/shop/issues/12', title: 'Fix CI', ticketKey: 'ALP-2' })} />,
    );
    const card = screen.getByRole('region', { name: 'Opened an issue' });
    expect(within(card).getByRole('link', { name: 'acme/shop#12' })).toHaveAttribute('href', 'https://github.com/acme/shop/issues/12');
    expect(card).toHaveTextContent('Fix CI');
    expect(pills(card)).toEqual(['ALP-2']);
    rerender(<PdtToolCard call={done('report_bug', { title: 'Crash on start' }, { kind: 'ticket', key: 'PDT-9', id: 't', projectId: 'p' })} />);
    const bug = screen.getByRole('region', { name: 'Reported a bug in PDT' });
    expect(pills(bug)).toEqual(['PDT-9']);
    expect(bug).toHaveTextContent('Crash on start');
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__issue_create', input: { title: 'Flaky' }, status: 'running' }} />);
    expect(screen.getByRole('region', { name: 'Opening an issue…' })).toHaveTextContent('Flaky');
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__report_bug', input: { title: 'x' }, status: 'failed', output: 'GitHub is down' }} />);
    expect(screen.getByRole('region', { name: "Didn't report the bug" })).toHaveTextContent('GitHub is down');
  });

  it('link_issue and unlink_issue: the ticket and the issue', () => {
    stubApi({});
    const issue = { key: 'o/r#1', url: 'https://github.com/o/r/issues/1', title: 'Crash on start' };
    const { rerender } = renderWithQuery(<PdtToolCard call={done('link_issue', { ticket: 'ALP-1', issue: issue.url }, { linked: true, issue, ticket: { key: 'ALP-1' } })} />);
    const card = screen.getByRole('region', { name: 'Linked an issue to ALP-1' });
    expect(pills(card)).toEqual(['ALP-1']);
    expect(within(card).getByRole('link', { name: 'o/r#1' })).toHaveAttribute('href', issue.url);
    expect(card).toHaveTextContent('Crash on start');
    rerender(<PdtToolCard call={codex('unlink_issue', { ticket: 'ALP-1', issue: 'o/r#1' }, { unlinked: true, issue, ticket: { key: 'ALP-1' } })} />);
    expect(screen.getByRole('region', { name: 'Unlinked an issue from ALP-1' })).toHaveTextContent('Crash on start');
  });

  it('documents: the name, kind, what changed, its links and Open beside', async () => {
    const calls = stubApi({ 'POST /api/sessions/pm1/documents': { tabs: [] } });
    const brief = { id: 'd1', title: 'Checkout spec', kind: 'doc', provider: 'local', url: null, projects: ['ALP'], tickets: ['ALP-3'] };
    const edit = done('doc_edit', { document: 'd1', edits: [{ op: 'replace_section', heading: 'Scope', markdown: 'x' }] }, { ...brief, version: '2', diff: '--- a\n+++ b\n@@ -1 +1 @@\n-Old.\n+New.\n' });
    const { rerender } = renderWithQuery(
      <MarkdownRefsContext.Provider value={{ sessionId: 'pm1', projectKey: 'ALP', worktreeId: null, forge: null }}>
        <PdtToolCard call={edit} />
      </MarkdownRefsContext.Provider>,
    );
    const card = screen.getByRole('region', { name: 'Edited Checkout spec' });
    expect(within(card).getByRole('link', { name: 'Checkout spec' })).toHaveAttribute('href', '/documents/d1');
    expect(card).toHaveTextContent('Doc');
    expect(card).toHaveTextContent('Replaced the section \u201cScope\u201d');
    expect(card).toHaveTextContent('Linked to');
    expect(pills(card)).toEqual(['ALP-3']);
    await userEvent.click(within(card).getByRole('button', { name: 'Open beside' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions/pm1/documents')).toBe(true));

    rerender(<PdtToolCard call={codex('document_create', { title: 'Budget', kind: 'sheet', project: 'ALP', openBeside: true }, { ...brief, title: 'Budget', kind: 'sheet', tickets: [], openBeside: true })} />);
    expect(screen.getByRole('region', { name: 'Created Budget' })).toHaveTextContent('Sheet');
    rerender(<PdtToolCard call={done('document_link', { document: 'd1', ticket: 'ALP-4' }, { ...brief, tickets: ['ALP-3', 'ALP-4'] })} />);
    expect(pills(screen.getByRole('region', { name: 'Linked Checkout spec' }))).toEqual(['ALP-3', 'ALP-4']);
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__sheet_edit', input: { document: 'Budget', edits: [{ op: 'add_chart' }] }, status: 'running' }} />);
    expect(screen.getByRole('region', { name: 'Editing Budget' })).toHaveTextContent('Added a chart');
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__document_create', input: { title: 'Plan', kind: 'doc' }, status: 'failed', output: 'no provider' }} />);
    expect(screen.getByRole('region', { name: "Didn't create Plan" })).toHaveTextContent('no provider');
  });

  it('save_note and delete_note: the title (body folded), its project and pinned', async () => {
    stubApi({});
    const { rerender } = renderWithQuery(
      <PdtToolCard call={done('save_note', { title: 'Deploys', body: 'Keys are in the **vault**.', project: 'ALP', pinned: true }, { saved: true, id: 'n1', title: 'Deploys', project: 'ALP', pinned: true })} />,
    );
    const card = screen.getByRole('region', { name: 'Saved a note' });
    expect(card).toHaveTextContent('ALP');
    expect(card).toHaveTextContent('Pinned');
    expect(within(card).queryByText('vault')).not.toBeInTheDocument();
    await userEvent.click(within(card).getByRole('button', { name: /Deploys/ }));
    expect(within(card).getByText('vault').tagName).toBe('STRONG');
    rerender(<PdtToolCard call={codex('delete_note', { id: 'n2' }, { deleted: true, title: 'Old rule', project: null, pinned: false })} />);
    const gone = screen.getByRole('region', { name: 'Deleted a note' });
    expect(gone).toHaveTextContent('Old rule');
    expect(gone).toHaveTextContent('Host-wide');
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__save_note', input: { id: 'n1', title: 'Deploys v2' }, status: 'running' }} />);
    expect(screen.getByRole('region', { name: 'Updating a note…' })).toHaveTextContent('Deploys v2');
    rerender(<PdtToolCard call={{ tool: 'mcp__pdt__delete_note', input: { id: 'n3' }, status: 'failed', output: "note n3 is the user's own" }} />);
    expect(screen.getByRole('region', { name: "Didn't delete the note" })).toHaveTextContent("the user's own");
  });

  it('a folded lookup names its tickets as pills, outside its toggle', async () => {
    stubApi({});
    renderWithQuery(<ToolCallRow verb="Read" tags={<TicketPills keys={['ALP-1', 'ALP-2', 'ALP-3', 'ALP-4', 'ALP-5', 'ALP-6', 'ALP-7', 'ALP-8']} />}><span>calls</span></ToolCallRow>);
    expect(pills(document.body)).toEqual(['ALP-1', 'ALP-2', 'ALP-3', 'ALP-4', 'ALP-5', 'ALP-6']);
    expect(screen.getByTitle('ALP-7, ALP-8')).toHaveTextContent('+2');
    expect(screen.getByRole('button', { name: 'Read' })).not.toContainElement(screen.getByRole('link', { name: 'ALP-1' }));
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
