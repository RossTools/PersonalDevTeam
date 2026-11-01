import { useState } from 'react';
import { ConfirmProvider } from '@ui/Confirm';
import { SegmentedControl } from '@ui/SegmentedControl';
import type { InboxItem } from '../../api/gen/InboxItem';
import { InboxList } from '../../inbox/InboxList';
import { InboxView } from '../../inbox/InboxView';

const ago = (min: number) => Date.now() - min * 60_000;

const base = (id: string, over: Partial<InboxItem>): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: null, ticketKey: 'SHOP-22',
  ticketTitle: 'Order confirmation email', attemptId: null, sessionId: null, runId: null, stepId: null, personaId: null,
  personaName: null, changeRequest: null, title: id, body: {}, choices: [], resolution: null, createdAt: ago(10),
  resolvedAt: null, ...over,
});

// One of each style the Inbox lays out (PDT-361): questions, passes, a notice, an approval, a stuck step, figures.
const items: InboxItem[] = [
  base('question', {
    kind: 'question', group: 'question', personaName: 'Coder', title: 'Should the receipt attach a PDF invoice?',
    body: { questions: [
      { question: "Should the receipt attach a PDF invoice, or is the email's text enough?", choices: ['Attach a PDF', 'Text is enough'] },
      { question: 'Which address should receipts come from?', choices: ['orders@acme.shop', 'no-reply@acme.shop'] },
    ] },
  }),
  base('approval', {
    kind: 'approval', group: 'approval', ticketKey: 'SHOP-23', ticketTitle: 'Guest checkout', personaName: 'QA', createdAt: ago(2),
    title: 'npm run db:reset && npm run db:seed', body: { tool: 'Bash', sessionTitle: 'QA · SHOP-23', description: 'Reset the dev database before testing' },
    choices: [{ id: 'allow', title: 'Allow', primary: true }, { id: 'allow_always', title: 'Always', primary: false }, { id: 'deny', title: 'Deny', primary: false }],
  }),
  base('stalled', {
    kind: 'stalled', group: 'stalled', ticketKey: 'SHOP-19', ticketTitle: 'Save the cart across visits', personaName: 'Coder', createdAt: ago(25),
    title: 'Coder stopped without an outcome', body: { why: 'It was reminded once and stopped again.' },
    choices: [{ id: 'nudge', title: 'Nudge', primary: true }, { id: 'outcome:done', title: 'Take over: done', primary: false }],
  }),
  base('loop', {
    kind: 'loop_limit', group: 'loop_limit', ticketKey: 'SHOP-17', ticketTitle: 'Shipping rates by region', createdAt: ago(60),
    title: 'Review sent it back 3 times: one more pass?', body: { workflow: 'Pipeline', limit: 3, passes: [
      { persona: 'Reviewer', outcome: 'changes_requested', summary: 'Rates are hard-coded; read them from the config.' },
      { persona: 'Reviewer', outcome: 'changes_requested', summary: 'Config read, but EU rates round the wrong way.' },
      { persona: 'Reviewer', outcome: 'approved', summary: 'Rounding fixed everywhere.' },
    ] },
    choices: [{ id: 'again', title: 'Once more', primary: true }, { id: 'stop', title: 'Stop', primary: false }],
  }),
  base('notice', {
    kind: 'notice', group: 'notice', attemptId: 'a', createdAt: ago(0), title: 'CI failed on PR #2 and no agent was there to fix it',
    body: { workflow: 'PR notices', text: '**1 check failed on a1b2c3d**\n\n- **test (ubuntu)** · npm test · 1 of 18 failed' },
  }),
  base('disk', {
    kind: 'low_disk', group: 'notice', ticketKey: null, ticketTitle: null, createdAt: ago(180), title: 'Disk space is low: 9 GB free',
    body: { freeBytes: 9 * 1024 ** 3, folders: [], text: 'The disk has 9 GB free. Worktrees’ build output is usually what fills it.' },
  }),
  base('accept', {
    group: 'accept', ticketKey: 'SHOP-24', ticketTitle: 'Apply discount codes at checkout', createdAt: ago(20),
    title: 'SHOP-24 passed review and QA. Accept it?', body: { workflow: 'Pipeline' },
    choices: [{ id: 'merge', title: 'Accept and merge', primary: true }, { id: 'back', title: 'Send back to the Coder', primary: false }, { id: 'wont_do', title: "Won't do", primary: false }],
  }),
];

function InboxDemo() {
  const [selected, setSelected] = useState<string | null>('question');
  return (
    <div className="flex flex-col gap-3">
      <SegmentedControl<string> aria-label="Item" value={selected ?? ''} onChange={setSelected}
        options={items.map((i) => ({ value: i.id, label: i.id }))} />
      <div className="grid h-[44rem] grid-cols-[18rem_minmax(0,1fr)] overflow-hidden rounded-md border border-border bg-background">
        <div className="overflow-auto border-r border-border bg-card">
          <InboxList items={items} selectedId={selected} onSelect={setSelected} />
        </div>
        <div className="flex min-h-0 flex-col">
          <InboxView items={items} selectedId={selected} onSelect={setSelected} />
        </div>
      </div>
    </div>
  );
}

export const section = { title: 'Inbox items', order: 55, render: () => <ConfirmProvider><InboxDemo /></ConfirmProvider> };
