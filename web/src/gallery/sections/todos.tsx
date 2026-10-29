import { useRef, useState, type ReactNode } from 'react';
import { Button, Calendar } from '@ui';
import type { Project } from '../../api/gen/Project';
import type { Todo } from '../../api/gen/Todo';
import { applyDuePatch, DuePicker, type DuePanel, type DueValue } from '../../todos/DuePicker';
import { addDays, dueGroup, localDay } from '../../todos/model';
import { QuickAdd } from '../../todos/QuickAdd';
import { TodoRow } from '../../todos/TodoRow';

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
  projectKey: null, ticketKey: null, ticketTitle: null, body: '', dueOn: null, dueTime: null, recurrence: null, ...more,
});

const ROWS: [string, Todo][] = [
  ['Overdue', todo('a', 'Renew the passport', { dueOn: addDays(TODAY, -2) })],
  ['Today, with a time and a ticket', todo('b', 'Review the login fix before standup', {
    dueOn: TODAY, dueTime: '23:59', projectId: 'p1', projectKey: 'PDT', ticketId: 't1', ticketKey: 'PDT-12', ticketTitle: 'Fix the login',
  })],
  ['Tomorrow, with a project', todo('c', 'Call Sam about the release', { dueOn: addDays(TODAY, 1), projectId: 'p1', projectKey: 'PDT' })],
  ['Next 7 days, repeating', todo('g', 'Water the plants', { dueOn: addDays(TODAY, 4), recurrence: { rule: { every: 'week', interval: 1, days: [1, 4] }, from: 'due' } })],
  ['Later, with notes', todo('d', 'Write the quarterly plan', { dueOn: addDays(TODAY, 12), body: '## Outline\n\n- Goals\n- Hiring' })],
  ['No date, a long title', todo('e', 'A long title that wraps onto a second line so the row shows how the meta line sits under it')],
  ['Done', todo('f', 'Pay the rent', { done: true, doneAt: 1, dueOn: addDays(TODAY, -1) })],
];

function Row({ initial, completing, highlighted }: { initial: Todo; completing?: boolean; highlighted?: boolean }) {
  const [t, setT] = useState(initial);
  return (
    <TodoRow
      todo={t}
      now={NOW}
      group={t.done && !completing ? null : dueGroup({ ...t, done: false }, NOW)}
      showProject
      projects={PROJECTS}
      completing={completing}
      highlighted={highlighted}
      onPatch={(p) => setT((x) => ({
        ...x,
        ...applyDuePatch(x, p),
        ...(p.title !== undefined ? { title: p.title } : {}),
        ...(p.done !== undefined ? { done: p.done } : {}),
      }))}
      onDelete={() => {}}
      onLink={() => {}}
      onRepeat={() => {}}
      onOpenTicket={() => {}}
      {...(t.done ? {} : { onMove: () => {}, onDropHere: () => {} })}
    />
  );
}

function Picker({ panel, label }: { panel: DuePanel; label: string }) {
  const [open, setOpen] = useState(false);
  const [v, setV] = useState<DueValue>({ dueOn: addDays(TODAY, 4), dueTime: '15:00', recurrence: { rule: { every: 'week', interval: 1, days: [1, 4] }, from: 'due' } });
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <>
      <Button ref={anchor} size="sm" onClick={() => setOpen(true)}>{label}</Button>
      <DuePicker open={open} onOpenChange={setOpen} anchor={anchor} panel={panel} value={v} now={NOW} label="Water the plants" onChange={(p) => setV((x) => applyDuePatch(x, p))} onCustomRepeat={() => {}} />
    </>
  );
}

function CalendarDemo() {
  const [day, setDay] = useState(addDays(TODAY, 4));
  return <Calendar value={day} min={TODAY} onChange={setDay} className="w-72" />;
}

function render() {
  return (
    <div className="flex flex-wrap items-start gap-8">
      <div className="flex w-80 flex-col gap-4">
        <Demo label="Quick add (focus it; type “call Sam tomorrow 3pm every month”)">
          <QuickAdd now={NOW} projects={PROJECTS} projectId={null} onAdd={async () => true} onCustomRepeat={() => {}} />
        </Demo>
        {ROWS.map(([label, t]) => (
          <Demo key={t.id} label={label}>
            <ul className="flex flex-col"><Row initial={t} /></ul>
          </Demo>
        ))}
        <Demo label="Completing (just ticked)">
          <ul className="flex flex-col"><Row initial={todo('h', 'Reply to Sam about the release date', { done: true, dueOn: TODAY })} completing /></ul>
        </Demo>
        <Demo label="Highlighted (opened from a reminder)">
          <ul className="flex flex-col"><Row initial={todo('i', 'Opened from the Inbox', { dueOn: TODAY })} highlighted /></ul>
        </Demo>
        <Demo label="Rows with separators">
          <ul className="flex flex-col">
            <Row initial={todo('j', 'Read the sqlx 0.9 changelog')} />
            <Row initial={todo('k', 'Try the new terminal font')} />
            <Row initial={todo('l', 'Book the train')} />
          </ul>
        </Demo>
      </div>
      <div className="flex flex-col gap-4">
        <Demo label="Due picker">
          <div className="flex gap-2">
            <Picker panel="date" label="Date" />
            <Picker panel="time" label="Time" />
            <Picker panel="repeat" label="Repeat" />
          </div>
        </Demo>
        <Demo label="Calendar">
          <CalendarDemo />
        </Demo>
      </div>
    </div>
  );
}

export const section = { title: 'Todos', order: 47, render };
