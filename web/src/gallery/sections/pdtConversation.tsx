import { useState, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ApprovalCard } from '@ui/ApprovalCard';
import { toast } from '@ui/Toaster';
import type { AttemptSummary } from '../../api/gen/AttemptSummary';
import { Markdown } from '../../common/Markdown';
import { MarkdownRefsContext, type MarkdownRefs } from '../../common/markdownRefs';
import { TicketKeysContext } from '../../common/ticketKeys';
import { DecideCard } from '../../conversation/DecideCard';
import { CreateTicketsCard, PdtToolCard, TicketPills, type PdtCall } from '../../conversation/PdtToolCard';
import { ToolCallRow } from '@ui/ToolCallRow';
import { PdtProposal } from '../../conversation/PdtProposal';
import { ticketKeys } from '../../tickets/queries';
import { sessionKeys } from '../../sessions/queries';
import { workflowKeys } from '../../workflows/queries';
import { watchKeys } from '../../common/Watch';
import { ticket } from '../../tickets/fixtures';
import { SectionLabel } from '@ui/SectionLabel';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      {children}
    </div>
  );
}

const refs: MarkdownRefs = {
  sessionId: 'gallery',
  projectKey: 'SHOP',
  worktreeId: null,
  forge: { kind: 'github', name: 'GitHub', repo: 'acme/shop', repoUrl: 'https://github.com/acme/shop', branchUrl: null, compareUrl: null },
};

const attempt: AttemptSummary = {
  id: 'a1', branch: 'pdt-SHOP-19-save-the-cart', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null,
  pr: {
    number: 7, url: 'https://github.com/acme/shop/pull/7', title: 'Save the cart', state: 'open', draft: false, baseBranch: 'main', headSha: 'c3d',
    mergeable: 'mergeable', behind: false, autoMerge: false, inMergeQueue: false, queuePosition: null,
    ci: { state: 'failed', url: '#', runs: [] }, queueCi: null, checkedAt: 1, error: null, local: false, mergeHeld: false,
  },
};

const watch = { id: 'w1', target: 'ticket', targetId: 't31', targetLabel: 'SHOP-31', targetTitle: 'Checkout page', targetState: 'in_progress', workflowId: null, sessionId: 'pm', watcher: 'PM', events: ['finished', 'failed', 'needs_you'], once: true, createdAt: 1 };

/** What the cards and hover cards would fetch, so the gallery shows them without a daemon. */
function Seeded({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  useState(() => {
    qc.setQueryData(ticketKeys.card('SHOP-19'), ticket(19, {
      key: 'SHOP-19', title: 'Save the cart across visits', stage: 'in_progress', priority: 'medium', labels: ['agentic'], attempt,
      agent: { persona: 'Coder', state: 'stalled', sessionId: 's', busy: false, runId: 'r', background: [] },
    }));
    qc.setQueryData(ticketKeys.card('SHOP-31'), ticket(31, { key: 'SHOP-31', title: 'Guest checkout: email-only orders', stage: 'todo', priority: 'high', labels: ['agentic'] }));
    for (const f of ['e2e/cart.spec.ts', 'src/checkout/session.ts']) qc.setQueryData(sessionKeys.file('gallery', f), true);
    qc.setQueryData(workflowKeys.run('r1'), { id: 'r1', workflowId: 'wf1', workflowName: 'Cut a release', status: 'running', error: null, steps: [] });
    qc.setQueryData(watchKeys.session('pm'), [watch]);
    return true;
  });
  return (
    <TicketKeysContext.Provider value={['SHOP']}>
      <MarkdownRefsContext.Provider value={refs}>{children}</MarkdownRefsContext.Provider>
    </TicketKeysContext.Provider>
  );
}

const ANSWER = [
  "**The cart ticket.** SHOP-19 isn't stuck on code: #7 fails in `e2e/cart.spec.ts` because the sandbox can't start Chrome",
  "(last push 4427552e). The checkout lives in `src/checkout/session.ts`; `src/gone.ts` isn't a file here. Hover a key for its card.",
  '',
  '| Ticket | What | State |',
  '| --- | --- | --- |',
  '| SHOP-31 | Guest checkout | Coder working |',
  '| SHOP-32 | Link guest orders | blocked by SHOP-31 |',
  '| SHOP-19 | Save the cart | CI failed |',
  '| SHOP-24 | Discount codes | ready to accept |',
].join('\n');

const doc = { id: 'd1', title: 'Guest checkout spec', kind: 'doc', provider: 'local', url: null, projects: ['SHOP'], tickets: ['SHOP-31'] };

const done = (tool: string, input: unknown, result: unknown): PdtCall => ({ tool: `mcp__pdt__${tool}`, input, status: 'done', output: JSON.stringify(result) });

export const section = {
  title: 'Conversation · PDT references and actions',
  order: 76,
  render: () => (
    <Seeded>
      <div className="flex max-w-3xl flex-col gap-6">
        <Demo label="Markdown in a conversation · ticket pills (hover: card), PR pills, file links, SHAs, a status table">
          <Markdown>{ANSWER}</Markdown>
        </Demo>
        <Demo label="ApprovalCard · a PDT tool says what it will create">
          <ApprovalCard
            title="Create this ticket?"
            canAlways
            preview={
              <PdtProposal
                tool="mcp__pdt__create_ticket"
                input={{
                  project: 'SHOP', title: 'Guest checkout: email-only orders', stage: 'todo', priority: 'high', labels: ['agentic'],
                  description: 'Orders without an account: email at checkout, no password. Done when a guest can pay and gets a receipt.',
                  reason: 'Guests drop off at sign-up.',
                }}
              />
            }
            onAnswer={(a) => void toast.info(`Answer: ${JSON.stringify(a)}`)}
          />
        </Demo>
        <Demo label="ApprovalCard · the PM's decisions as rows to untick">
          <DecideCard
            id="gallery-decide"
            keyboard={false}
            rows={[
              { itemId: 'i1', ticketId: 't1', ticketKey: 'SHOP-41', ticketTitle: 'Flaky checkout test', decision: 'accept_todo', agentic: true, priority: 'high' },
              { itemId: 'i2', ticketId: 't2', ticketKey: 'SHOP-42', ticketTitle: 'Same flaky test', decision: 'duplicate', agentic: false, duplicateOf: 'SHOP-41' },
              { itemId: 'i3', ticketId: 't3', ticketKey: 'SHOP-43', ticketTitle: 'Rename everything', decision: 'reject', agentic: false, note: 'Not worth the churn.' },
            ]}
            onAnswer={async (a, kept) => void toast.info(`Answer: ${JSON.stringify(a)}${kept ? `, rows ${kept.join(', ')}` : ''}`)}
          />
        </Demo>
        <Demo label="PDT tool cards · created, changed, linked, message, comment, workflow run, watch, running, failed">
          <div className="flex flex-col gap-3">
            <CreateTicketsCard calls={[
              done('create_ticket', { title: 'Guest checkout: email-only orders', stage: 'todo', labels: ['agentic'] }, { key: 'SHOP-31', stage: 'todo' }),
              done('create_ticket', { title: 'Link guest orders when the guest signs up later', labels: ['agentic'] }, { key: 'SHOP-32', stage: 'draft' }),
            ]} />
            <PdtToolCard call={done('edit_ticket', { ticket: 'SHOP-19' }, { key: 'SHOP-19', changes: [{ field: 'stage', from: 'todo', to: 'in_progress' }, { field: 'priority', from: 'low', to: 'high' }, { field: 'description' }] })} />
            <PdtToolCard call={done('link_tickets', { ticket: 'SHOP-32', kind: 'blockedBy', other: 'SHOP-31' }, { linked: true })} />
            <PdtToolCard call={done('send_to_session', { session: 's', text: "Skip the e2e run locally (the sandbox has no Chrome); CI runs it. Commit and submit your outcome." }, { queued: true, to: 'Coder · SHOP-19' })} />
            <PdtToolCard call={done('add_ticket_comment', { ticket: 'SHOP-19', body: 'The e2e run needs Chrome, which the sandbox lacks: **CI runs it**.' }, { commented: true, key: 'SHOP-19' })} />
            <PdtToolCard call={done('workflow_run', { name: 'Cut a release', project: 'SHOP' }, { runId: 'r1', workflow: 'Cut a release' })} />
            <PdtToolCard call={done('watch', { ticket: 'SHOP-31' }, { watching: [watch] })} />
            <PdtToolCard call={{ tool: 'mcp__pdt__create_ticket', input: { title: 'Discount codes' }, status: 'running' }} />
            <PdtToolCard call={{ tool: 'mcp__pdt__link_tickets', input: { ticket: 'SHOP-1', kind: 'blocks', other: 'SHOP-1' }, status: 'failed', output: 'a ticket cannot block itself' }} />
          </div>
        </Demo>
        <Demo label="PDT tool cards · stopped work, issues, documents, notes (running and failed too)">
          <div className="flex flex-col gap-3">
            <PdtToolCard call={done('stop_ticket', { ticket: 'SHOP-19' }, { stopped: true, key: 'SHOP-19', stage: 'todo', runsCancelled: 1, sessionsStopped: 1, worktreeRemoved: false, branchNote: null })} />
            <PdtToolCard call={done('issue_create', { ticket: 'SHOP-19' }, { repo: 'acme/shop', number: 41, issue: 'acme/shop#41', url: 'https://github.com/acme/shop/issues/41', title: 'Save the cart across visits', ticketKey: 'SHOP-19' })} />
            <PdtToolCard call={done('report_bug', { title: 'The board forgets its filter after a restart' }, { kind: 'issue', number: 88, url: 'https://github.com/acme/pdt/issues/88' })} />
            <PdtToolCard call={done('link_issue', { ticket: 'SHOP-31', issue: 'https://github.com/acme/shop/issues/97' }, { linked: true, issue: { key: 'acme/shop#97', url: 'https://github.com/acme/shop/issues/97', title: 'Guests can’t check out' }, ticket: { key: 'SHOP-31' } })} />
            <PdtToolCard call={done('document_create', { title: 'Guest checkout spec', kind: 'doc', ticket: 'SHOP-31' }, { ...doc, title: 'Guest checkout spec' })} />
            <PdtToolCard call={done('doc_edit', { document: 'd1', edits: [{ op: 'replace_section', heading: 'Scope', markdown: '…' }, { op: 'table_append_row', heading: 'Open questions', cells: ['Receipts', 'PM'] }] }, { ...doc, version: '3', diff: '--- a\n+++ b\n@@ -1,2 +1,3 @@\n-Old scope.\n+Email-only orders.\n+No password.\n' })} />
            <PdtToolCard call={done('sheet_edit', { document: 'Q3 budget', edits: [{ op: 'set_cells', sheet: 'Costs', cell: 'B2', values: [['1200']] }, { op: 'add_chart' }] }, { ...doc, id: 'd2', title: 'Q3 budget', kind: 'sheet', tickets: [], version: '8', charts: ['c1'] })} />
            <PdtToolCard call={done('document_link', { document: 'd1', ticket: 'SHOP-32' }, { ...doc, title: 'Guest checkout spec', tickets: ['SHOP-31', 'SHOP-32'] })} />
            <PdtToolCard call={done('save_note', { title: 'Release on Tuesdays', body: 'The user cuts releases on **Tuesdays** only; never suggest a Friday release.', project: 'SHOP', pinned: true }, { saved: true, id: 'n1', title: 'Release on Tuesdays', project: 'SHOP', pinned: true })} />
            <PdtToolCard call={done('delete_note', { id: 'n2' }, { deleted: true, title: 'CI runs on the Mac mini', project: null, pinned: false })} />
            <PdtToolCard call={{ tool: 'mcp__pdt__sheet_edit', input: { document: 'Q3 budget', edits: [{ op: 'sort', sheet: 'Costs', range: 'A1:C9', by: [] }] }, status: 'running' }} />
            <PdtToolCard call={{ tool: 'mcp__pdt__issue_create', input: { title: 'Flaky e2e' }, status: 'failed', output: 'acme/shop has no issues' }} />
          </div>
        </Demo>
        <Demo label="ToolCallRow · folded lookups name the tickets they read">
          <div className="flex flex-col">
            <ToolCallRow verb="Read" label="Read SHOP-19, SHOP-31" tags={<TicketPills keys={['SHOP-19', 'SHOP-31']} />}>
              <ToolCallRow verb="pdt: get_ticket" status="done" />
              <ToolCallRow verb="pdt: get_ticket" status="done" />
            </ToolCallRow>
            <ToolCallRow verb="Read" status="done" tags={<TicketPills keys={['SHOP-19', 'SHOP-24', 'SHOP-31', 'SHOP-32', 'SHOP-33', 'SHOP-34', 'SHOP-35', 'SHOP-36']} />} details={<span>search_tickets</span>} />
          </div>
        </Demo>
      </div>
    </Seeded>
  ),
};
