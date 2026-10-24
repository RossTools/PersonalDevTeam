import { useState, type ReactNode } from 'react';
import type { Project } from '../../api/gen/Project';
import type { Todo } from '../../api/gen/Todo';
import { addDays, localDay } from '../../todos/model';
import { TodoCard } from '../../todos/TodoCard';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
      {children}
    </div>
  );
}

const NOW = new Date();
const TODAY = localDay(NOW);
const PROJECTS = [{ id: 'p1', key: 'PDT', name: 'Personal Dev Team' }] as Project[];
const todo = (id: string, title: string, more: Partial<Todo> = {}): Todo => ({
  id, title, position: 0, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1, doneAt: null,
  projectKey: null, ticketKey: null, ticketTitle: null, dueOn: null, dueTime: null, recurrence: null, ...more,
});

const CARDS: [string, Todo][] = [
  ['Overdue', todo('a', 'Renew the passport', { dueOn: addDays(TODAY, -2) })],
  ['Today, with a time and a ticket', todo('b', 'Review the login fix before standup', {
    dueOn: TODAY, dueTime: '23:59', projectId: 'p1', projectKey: 'PDT', ticketId: 't1', ticketKey: 'PDT-12', ticketTitle: 'Fix the login',
  })],
  ['Tomorrow', todo('c', 'Call Sam about the release', { dueOn: addDays(TODAY, 1), projectId: 'p1', projectKey: 'PDT' })],
  ['Later', todo('d', 'Write the quarterly plan', { dueOn: addDays(TODAY, 12) })],
  ['Repeating', todo('g', 'Water the plants', { dueOn: addDays(TODAY, 2), recurrence: { rule: { every: 'week', interval: 1, days: [1, 3] }, from: 'due' } })],
  ['No date', todo('e', 'A long title that wraps onto a second line so the card shows how the chips sit under it')],
  ['Done (no chip)', todo('f', 'Pay the rent', { done: true, doneAt: 1, dueOn: addDays(TODAY, -1) })],
];

function Card({ initial }: { initial: Todo }) {
  const [t, setT] = useState(initial);
  return (
    <ul className="flex flex-col">
      <TodoCard
        todo={t}
        now={NOW}
        showProject
        projects={PROJECTS}
        onPatch={(p) => setT((x) => ({
          ...x,
          ...(p.title !== undefined ? { title: p.title } : {}),
          ...(p.done !== undefined ? { done: p.done } : {}),
          ...(p.dueOn !== undefined ? { dueOn: p.dueOn || null } : {}),
        }))}
        onDelete={() => {}}
        onLink={() => {}}
        onPickDate={() => {}}
        onRepeat={() => {}}
        onOpenTicket={() => {}}
        {...(t.done ? {} : { onMove: () => {}, onDropHere: () => {} })}
      />
    </ul>
  );
}

function render() {
  return (
    <div className="grid max-w-xs gap-4">
      {CARDS.map(([label, t]) => (
        <Demo key={t.id} label={label}>
          <Card initial={t} />
        </Demo>
      ))}
      <Demo label="Highlighted (opened from a reminder)">
        <ul className="flex flex-col">
          <TodoCard todo={todo('g', 'Opened from the Inbox', { dueOn: TODAY })} now={NOW} showProject projects={PROJECTS} highlighted
            onPatch={() => {}} onDelete={() => {}} onLink={() => {}} onPickDate={() => {}} onRepeat={() => {}} onOpenTicket={() => {}} />
        </ul>
      </Demo>
    </div>
  );
}

export const section = { title: 'Todo card', order: 47, render };
