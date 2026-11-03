import { useState, type ReactNode } from 'react';
import { ActionBar } from '@ui/ActionBar';
import { AgentMessage, Avatar, PassList } from '@ui/AgentMessage';
import { Button } from '@ui/Button';
import { Callout } from '@ui/Callout';
import { CodeCard } from '@ui/CodeCard';
import { Facts } from '@ui/Facts';
import { Kbd } from '@ui/Kbd';
import { NO_ANSWER, QuestionCard, type QuestionAnswer } from '@ui/QuestionCard';
import { SectionLabel } from '@ui/SectionLabel';
import { Textarea } from '@ui/Textarea';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      {children}
    </div>
  );
}

function QuestionDemo() {
  const [first, setFirst] = useState<QuestionAnswer>(NO_ANSWER);
  const [second, setSecond] = useState<QuestionAnswer>(NO_ANSWER);
  return (
    <div className="flex flex-col gap-3">
      <QuestionCard
        id="gallery-q1" index={0} total={2} numbers
        question="Should the receipt attach a PDF invoice?" label="Should the receipt attach a PDF invoice?"
        choices={[
          { label: 'Attach a PDF', description: 'Customers can file it; one more template to keep.' },
          { label: 'Text is enough', description: 'Nothing new to build.' },
        ]}
        recommended="Text is enough" why="Nobody has asked for a PDF yet."
        value={first} onChange={setFirst}
      />
      <QuestionCard
        id="gallery-q2" index={1} total={2}
        question="What should the email's subject say?" label="What should the email's subject say?"
        choices={[]} value={second} onChange={setSecond}
      />
    </div>
  );
}

function render() {
  return (
    <div className="flex flex-col gap-6">
      <div className="grid gap-6 lg:grid-cols-2">
        <Demo label="Callout · info, success, warning, error">
          <Callout title="Runs share one port range">Each run gets the next free port from 8400–8499.</Callout>
          <Callout tone="success" title="Cleaned up">Freed 2.4 GB.</Callout>
          <Callout tone="warning" title="Disk almost full">9 GB free of 994 GB.</Callout>
          <Callout tone="error" title="The build failed">cargo exited with code 101.</Callout>
        </Demo>
        <Demo label="Facts">
          <Facts aria-label="Storage" facts={[
            { value: '9 GB', label: 'free of 994 GB' },
            { value: '2.4 GB', label: 'cleaning would free' },
            { value: '31', label: 'ended sessions' },
          ]} />
        </Demo>
      </div>
      <div className="grid gap-6 lg:grid-cols-2">
        <Demo label="CodeCard · a command, and a diff in its place">
          <CodeCard title="Bash · in shop-api" aside="Reset the dev database before testing" code="npm run db:reset && npm run db:seed" />
          <CodeCard title="Edit · src/cart.ts" aside="+1 −1">
            <div className="px-3.5 py-3 font-mono text-xs">
              <div className="text-failed">− const ttl = 3600;</div>
              <div className="text-running">+ const ttl = 30 * 24 * 3600;</div>
            </div>
          </CodeCard>
        </Demo>
        <Demo label="Avatar · AgentMessage">
          <div className="flex items-center gap-2">
            <Avatar name="Coder" /><Avatar name="Reviewer" /><Avatar name="QA" /><Avatar name="PM" size="sm" />
          </div>
          <AgentMessage name="Coder" label="Coder's context">
            The cart is kept in local storage for 30 days; signing in merges it into the account's cart.
          </AgentMessage>
        </Demo>
      </div>
      <div className="grid gap-6 lg:grid-cols-2">
        <Demo label="PassList">
          <PassList passes={[
            { name: 'Coder', title: 'Pass 1 · Coder', outcome: 'submitted', summary: 'Built the receipt email and its test.' },
            { name: 'Reviewer', title: 'Pass 1 · Reviewer', outcome: 'changes requested', tone: 'bad', summary: 'The subject line is missing the order number.' },
            { name: 'Reviewer', title: 'Pass 2 · Reviewer', outcome: 'approved', tone: 'good' },
            { name: 'QA', outcome: null, summary: 'Still running.' },
          ]} />
        </Demo>
        <Demo label="QuestionCard · choices (number keys, a recommended one), and free text">
          <QuestionDemo />
        </Demo>
      </div>
      <Demo label="ActionBar · note, buttons, hint, end">
        <div className="rounded-md border border-border">
          <ActionBar
            note={<Textarea aria-label="Note" placeholder="Add a note (optional)" rows={1} />}
            hint={<span className="flex items-center gap-1"><Kbd>Enter</Kbd> approves</span>}
            end={<Button variant="ghost">Dismiss</Button>}
          >
            <Button variant="primary">Approve</Button>
            <Button variant="secondary">Request changes</Button>
          </ActionBar>
        </div>
      </Demo>
    </div>
  );
}

export const section = { title: 'Cards and decisions', order: 25, render };
