// One todo as a card (PDT-140): checkbox, title (click to rename), project/ticket chips, its due chip, its repeat
// rule (PDT-141), and a hover menu. Drag (or Alt+↑/↓) reorders it within its group. Pure: the list hands it its actions.
import { useState, type DragEvent, type KeyboardEvent } from 'react';
import { CalendarDays, GripVertical, Repeat } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, Checkbox, dragData, MenuButton, parseDragData, type MenuEntry } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { ProjectKey } from '../common/BranchChip';
import { InlineRename } from '../common/InlineRename';
import { TicketKey } from '../common/TicketKey';
import { dueLabel, dueState, formatDay, quickDays, repeatText } from './model';

export const TODO_DRAG_KIND = 'todo';

/** The due chip: Overdue in red, Today, Tomorrow, otherwise the date. Nothing for done or undated todos. */
export function DueChip({ todo: t, now }: { todo: Pick<Todo, 'done' | 'dueOn' | 'dueTime'>; now: Date }) {
  const label = dueLabel(t, now);
  const state = dueState(t, now);
  if (!label || !t.dueOn) return null;
  const when = `Due ${formatDay(t.dueOn, now)}${t.dueTime ? ` at ${t.dueTime}` : ''}`;
  return (
    <Badge tone={state === 'overdue' ? 'failed' : state === 'today' ? 'accent' : 'neutral'} title={when} aria-label={when} className="gap-1">
      <CalendarDays aria-hidden className="size-3" />
      {label}
    </Badge>
  );
}

/** A repeating todo's rule: a repeat icon and "Every Mon, Wed". Nothing for done todos or ones that don't repeat. */
export function RepeatChip({ todo: t }: { todo: Pick<Todo, 'done' | 'recurrence'> }) {
  if (t.done || !t.recurrence) return null;
  const text = repeatText(t.recurrence);
  const title = `Repeats: ${text}${t.recurrence.from === 'done' ? ', counting from when it’s done' : ''}`;
  return (
    <Badge tone="neutral" title={title} aria-label={title} className="gap-1">
      <Repeat aria-hidden className="size-3" />
      {text}
    </Badge>
  );
}

/** The menu entries that set a due date: the quick picks (a day without a time), a date of the user's choosing, none. */
export function dueEntries(current: Pick<Todo, 'dueOn'>, now: Date, set: (dueOn: string) => void, pick: () => void): MenuEntry[] {
  return [
    ...quickDays(now).map((q) => ({
      label: `${q.day === current.dueOn ? '✓ ' : ''}${q.label} · ${formatDay(q.day, now)}`,
      onSelect: () => set(q.day),
    })),
    { label: 'Pick a date…', onSelect: pick },
    ...(current.dueOn ? [{ label: 'No due date', onSelect: () => set('') }] : []),
  ];
}

export function TodoCard({ todo: t, now, showProject, projects, highlighted, onPatch, onDelete, onLink, onPickDate, onRepeat, onOpenTicket, onMove, onDropHere }: {
  todo: Todo;
  now: Date;
  /** The All filter: show the project's chip. */
  showProject: boolean;
  projects: readonly Project[];
  /** Just opened from a notification: a ring for a moment. */
  highlighted?: boolean;
  onPatch: (p: TodoPatch) => void;
  onDelete: () => void;
  onLink: () => void;
  /** Opens the date picker. */
  onPickDate: () => void;
  /** Opens the repeat editor. */
  onRepeat: () => void;
  onOpenTicket: (t: { id: string; projectId: string }) => void;
  /** Open cards: Alt+↑/↓ moves the card past its neighbour in its group. */
  onMove?: (where: 'before' | 'after') => void;
  /** Open cards: another card was dropped on this one's top or bottom half. */
  onDropHere?: (id: string, where: 'before' | 'after') => void;
}) {
  const [editing, setEditing] = useState(false);
  const [over, setOver] = useState<'before' | 'after' | null>(null);
  const draggable = !!onDropHere && !editing;
  const half = (e: DragEvent<HTMLElement>): 'before' | 'after' => {
    const r = e.currentTarget.getBoundingClientRect();
    return e.clientY < r.top + r.height / 2 ? 'before' : 'after';
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (!onMove || !e.altKey || editing) return;
    if (e.key === 'ArrowUp') { e.preventDefault(); onMove('before'); }
    if (e.key === 'ArrowDown') { e.preventDefault(); onMove('after'); }
  };
  const entries = (): MenuEntry[] => [
    { label: 'Edit', onSelect: () => setEditing(true) },
    ...(t.done ? [] : [
      { submenu: 'Due date', icon: <CalendarDays aria-hidden />, entries: dueEntries(t, now, (dueOn) => onPatch({ dueOn, dueTime: '' }), onPickDate) },
      { label: 'Repeat…', icon: <Repeat aria-hidden />, onSelect: onRepeat },
      ...(t.recurrence ? [{ label: 'Stop repeating', onSelect: () => onPatch({ recurrence: null }) }] : []),
    ]),
    { label: t.ticketId ? 'Link to another ticket…' : 'Link to a ticket…', onSelect: onLink },
    ...(t.ticketId ? [{ label: `Unlink ${t.ticketKey ?? 'the ticket'}`, onSelect: () => onPatch({ ticket: '' }) }] : []),
    {
      submenu: 'Project',
      entries: [
        { label: t.projectId ? 'No project' : '✓ No project', onSelect: () => onPatch({ project: '' }) },
        ...projects.map((p) => ({ label: p.id === t.projectId ? `✓ ${p.key || p.name}` : p.key || p.name, onSelect: () => onPatch({ project: p.id }) })),
      ],
    },
    { separator: true },
    { label: 'Delete', danger: true, onSelect: onDelete },
  ];
  const chips = (showProject && t.projectKey && !t.ticketKey) || t.ticketKey || dueLabel(t, now) || (!t.done && t.recurrence);
  return (
    <li
      id={`todo-${t.id}`}
      aria-label={t.title}
      draggable={draggable}
      onDragStart={draggable ? (e) => {
        e.dataTransfer.setData('text/plain', dragData(TODO_DRAG_KIND, t.id));
        e.dataTransfer.effectAllowed = 'move';
      } : undefined}
      onDragOver={onDropHere ? (e) => { e.preventDefault(); setOver(half(e)); } : undefined}
      onDragLeave={() => setOver(null)}
      onDrop={onDropHere ? (e) => {
        setOver(null);
        const d = parseDragData(e.dataTransfer.getData('text/plain') ?? '');
        if (!d || d.kind !== TODO_DRAG_KIND || d.id === t.id) return;
        e.preventDefault();
        onDropHere(d.id, half(e));
      } : undefined}
      onKeyDown={onKeyDown}
      className={cn(
        'group/todo relative flex min-w-0 items-start gap-1.5 rounded-md border border-border bg-card py-1.5 pr-1 pl-0.5 text-sm',
        'transition-colors duration-(--pdt-duration) hover:border-input motion-reduce:transition-none',
        draggable && 'cursor-grab active:cursor-grabbing',
        highlighted && 'ring-2 ring-ring',
        t.done && 'bg-transparent',
      )}
    >
      {over && <span aria-hidden className={cn('pointer-events-none absolute inset-x-1 h-0.5 rounded-full bg-primary', over === 'before' ? '-top-1' : '-bottom-1')} />}
      {onDropHere ? (
        <GripVertical aria-hidden className="mt-1 size-3 shrink-0 text-muted-foreground opacity-0 group-hover/todo:opacity-100 pointer-coarse:hidden" />
      ) : <span aria-hidden className="w-3 shrink-0" />}
      <Checkbox
        aria-label={t.done ? `Not done: ${t.title}` : `Done: ${t.title}`}
        checked={t.done}
        onChange={(done) => onPatch({ done })}
        className="mt-0.5"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        {editing ? (
          <InlineRename
            label="Todo"
            initial={t.title}
            maxLength={500}
            onCommit={(title) => { setEditing(false); onPatch({ title }); }}
            onCancel={() => setEditing(false)}
          />
        ) : (
          <button
            type="button"
            title={onMove ? 'Click to edit · Alt+↑/↓ to reorder' : 'Click to edit'}
            onClick={() => setEditing(true)}
            className={cn(
              'min-w-0 rounded-sm text-left break-words outline-none focus-visible:ring-2 focus-visible:ring-ring',
              t.done && 'text-muted-foreground line-through',
            )}
          >
            {t.title}
          </button>
        )}
        {chips && (
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            <DueChip todo={t} now={now} />
            <RepeatChip todo={t} />
            {showProject && t.projectKey && !t.ticketKey && <ProjectKey projectKey={t.projectKey} />}
            {t.ticketKey && t.ticketId && t.projectId && (
              <TicketKey
                ticketKey={t.ticketKey}
                title={t.ticketTitle ?? undefined}
                onClick={() => onOpenTicket({ id: t.ticketId!, projectId: t.projectId! })}
              />
            )}
          </div>
        )}
      </div>
      <MenuButton label={`Actions for ${t.title}`} size="sm" entries={entries} className="shrink-0 opacity-0 group-hover/todo:opacity-100 focus-visible:opacity-100 data-pressed:opacity-100 pointer-coarse:opacity-100" />
    </li>
  );
}
