// One todo as a row (PDT-143, was PDT-140's card): a round checkbox, the title (click to open it, PDT-227; Edit in ⋯
// renames it in place), the first line of its notes, muted, and a meta line (due day or time, repeat rule, ticket,
// project, a notes icon). On hover a calendar button (the due picker) and ⋯. Drag (a handle on
// touch) or Alt+↑/↓ reorders it within its group. Pure: the list hands it its actions.
import { useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode } from 'react';
import { CalendarDays, Clock, GripVertical, NotebookText, Repeat } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Checkbox, dragData, IconButton, Link, MenuButton, parseDragData, type MenuEntry } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { ProjectKey } from '../common/BranchChip';
import { InlineRename } from '../common/InlineRename';
import { useMediaQuery } from '../common/useMediaQuery';
import { DuePicker, dayText } from './DuePicker';
import { bodyPreview, formatDay, repeatText, type DueGroupId } from './model';

const TODO_DRAG_KIND = 'todo';

/** The meta line's due part, by the row's group: the day where the heading doesn't say it, and the time. */
function dueMeta(t: Todo, group: DueGroupId | null, now: Date): { text: string; tone: 'overdue' | 'today' | null; time: boolean } | null {
  if (!t.dueOn || !group || group === 'none') return null;
  if (group === 'overdue') return { text: `${dayText(t.dueOn, now)}${t.dueTime ? ` ${t.dueTime}` : ''}`, tone: 'overdue', time: false };
  if (group === 'today' || group === 'tomorrow') return t.dueTime ? { text: t.dueTime, tone: group === 'today' ? 'today' : null, time: true } : null;
  return { text: `${formatDay(t.dueOn, now)}${t.dueTime ? ` ${t.dueTime}` : ''}`, tone: null, time: false };
}

export function TodoRow({ todo: t, now, group, showProject, projects, highlighted, completing, onOpen, onPatch, onDelete, onLink, onRepeat, onOpenTicket, onMove, onDropHere }: {
  todo: Todo;
  now: Date;
  /** The open group it is in (its meta line leaves out what the heading says); null when done. */
  group: DueGroupId | null;
  /** The All scope: show the project's key (when there's no ticket). */
  showProject: boolean;
  projects: readonly Project[];
  /** Just opened from a notification: a ring for a moment. */
  highlighted?: boolean;
  /** Just ticked: struck through on a faint green fill until it moves to Completed. */
  completing?: boolean;
  /** Opens it large (a click on the title); without it the click renames it in place. */
  onOpen?: () => void;
  onPatch: (p: TodoPatch) => void;
  onDelete: () => void;
  onLink: () => void;
  /** Opens the full repeat editor (Custom…, Repeat…). */
  onRepeat: () => void;
  onOpenTicket: (t: { id: string; projectId: string }) => void;
  /** Open rows: Alt+↑/↓ moves the row past its neighbour in its group. */
  onMove?: (where: 'before' | 'after') => void;
  /** Open rows: another row was dropped on this one's top or bottom half. */
  onDropHere?: (id: string, where: 'before' | 'after') => void;
}) {
  const [editing, setEditing] = useState(false);
  const [over, setOver] = useState<'before' | 'after' | null>(null);
  const [picking, setPicking] = useState(false);
  const anchor = useRef<Element | null>(null);
  const dueRef = useRef<HTMLButtonElement>(null);
  const calRef = useRef<HTMLDivElement>(null);
  const actsRef = useRef<HTMLDivElement>(null);
  const coarse = useMediaQuery('(pointer: coarse)');
  const canDrag = !!onDropHere && !editing && !completing;
  const pick = (from: Element | null) => {
    anchor.current = from;
    setPicking(true);
  };
  const half = (e: DragEvent<HTMLElement>): 'before' | 'after' => {
    const r = e.currentTarget.getBoundingClientRect();
    return e.clientY < r.top + r.height / 2 ? 'before' : 'after';
  };
  const startDrag = (e: DragEvent<HTMLElement>) => {
    e.dataTransfer.setData('text/plain', dragData(TODO_DRAG_KIND, t.id));
    e.dataTransfer.effectAllowed = 'move';
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (!onMove || !e.altKey || editing) return;
    if (e.key === 'ArrowUp') { e.preventDefault(); onMove('before'); }
    if (e.key === 'ArrowDown') { e.preventDefault(); onMove('after'); }
  };
  const entries = (): MenuEntry[] => [
    ...(onOpen ? [{ label: 'Open', onSelect: onOpen }] : []),
    { label: 'Edit', onSelect: () => setEditing(true) },
    ...(t.done ? [] : [
      { label: 'Due date…', icon: <CalendarDays aria-hidden />, onSelect: () => pick(actsRef.current) },
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
  const due = t.done ? null : dueMeta(t, group, now);
  const overdue = group === 'overdue' && !t.done;
  const repeat = !t.done && t.recurrence ? repeatText(t.recurrence) : null;
  const project = showProject && t.projectKey && !t.ticketKey ? t.projectKey : null;
  const ticket = t.ticketKey && t.ticketId && t.projectId ? t.ticketKey : null;
  const preview = bodyPreview(t.body);
  const hasNotes = t.body.trim() !== '';
  const when = t.dueOn ? `Due ${formatDay(t.dueOn, now)}${t.dueTime ? ` at ${t.dueTime}` : ''}` : '';
  return (
    <li
      id={`todo-${t.id}`}
      aria-label={t.title}
      data-completing={completing || undefined}
      draggable={canDrag && !coarse}
      onDragStart={canDrag && !coarse ? startDrag : undefined}
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
        'group/todo relative flex min-w-0 items-start gap-2.5 rounded-lg px-2.5 py-2 text-sm',
        // A hairline between rows, from the title on; gone around the row under the pointer.
        'before:pointer-events-none before:absolute before:top-0 before:right-2.5 before:left-9.5 before:h-px before:bg-border first:before:hidden',
        'hover:bg-surface-2 hover:before:opacity-0 [li:hover+&]:before:opacity-0',
        'transition-colors duration-(--pdt-duration) motion-reduce:transition-none',
        canDrag && !coarse && 'cursor-grab active:cursor-grabbing',
        highlighted && 'ring-2 ring-ring',
        completing && 'bg-running-soft before:opacity-0 hover:bg-running-soft [li[data-completing]+&]:before:opacity-0',
      )}
    >
      {over && <span aria-hidden className={cn('pointer-events-none absolute inset-x-2 h-0.5 rounded-full bg-primary', over === 'before' ? '-top-px' : '-bottom-px')} />}
      {canDrag && coarse && (
        <span draggable onDragStart={startDrag} aria-hidden className="-ml-2 flex min-h-control-touch w-5 shrink-0 cursor-grab touch-none items-center justify-center self-stretch text-muted-foreground">
          <GripVertical className="size-3.5" />
        </span>
      )}
      <Checkbox
        aria-label={t.done ? `Not done: ${t.title}` : `Done: ${t.title}`}
        shape="round"
        danger={overdue}
        checked={t.done}
        onChange={(done) => onPatch({ done })}
        className="mt-px"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
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
            title={`${onOpen ? 'Click to open' : 'Click to edit'}${onMove ? ' · Alt+↑/↓ to reorder' : ''}`}
            onClick={onOpen ?? (() => setEditing(true))}
            className={cn(
              'min-w-0 rounded-sm text-left leading-5 break-words outline-none focus-visible:ring-2 focus-visible:ring-ring',
              (t.done || completing) && 'text-muted-foreground line-through decoration-muted-foreground',
            )}
          >
            {t.title}
          </button>
        )}
        {preview && (
          <p className="truncate text-xs leading-4 text-muted-foreground">{preview}</p>
        )}
        {(due || repeat || project || ticket || hasNotes) && (
          <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
            {due && (
              <button
                ref={dueRef}
                type="button"
                title={when}
                aria-label={`${when}: change`}
                onClick={() => pick(dueRef.current)}
                className={cn(
                  'inline-flex items-center gap-1 rounded-sm outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring',
                  due.tone === 'overdue' && 'text-destructive',
                  due.tone === 'today' && 'text-waiting',
                )}
              >
                {due.time ? <Clock aria-hidden className="size-3" /> : <CalendarDays aria-hidden className="size-3" />}
                {due.text}
              </button>
            )}
            {repeat && t.recurrence && (
              <Meta icon={<Repeat aria-hidden className="size-3" />} title={`Repeats: ${repeat}${t.recurrence.from === 'done' ? ', counting from when it’s done' : ''}`}>
                {repeat.replace(/^Every (?=[A-Z][a-z]{2}, )/, '')}
              </Meta>
            )}
            {ticket && (
              <Link
                tone="quiet"
                title={t.ticketTitle ?? undefined}
                onClick={() => onOpenTicket({ id: t.ticketId!, projectId: t.projectId! })}
                className="font-mono text-meta text-foreground/80"
              >
                {ticket}
              </Link>
            )}
            {project && <ProjectKey projectKey={project} />}
            {hasNotes && <Meta icon={<NotebookText aria-hidden className="size-3" />} title="Has notes">{null}</Meta>}
          </div>
        )}
      </div>
      <div ref={actsRef} className="-mt-0.5 -mr-1 flex shrink-0 items-center gap-0.5 opacity-0 group-hover/todo:opacity-100 focus-within:opacity-100 has-data-pressed:opacity-100 pointer-coarse:opacity-100">
        {!t.done && (
          <div ref={calRef} className="pointer-coarse:hidden">
            <IconButton label={`Due date for ${t.title}`} size="sm" onClick={() => pick(calRef.current)} className="text-muted-foreground">
              <CalendarDays aria-hidden />
            </IconButton>
          </div>
        )}
        <MenuButton label={`Actions for ${t.title}`} size="sm" entries={entries} className="text-muted-foreground" />
      </div>
      {picking && (
        <DuePicker
          open
          onOpenChange={setPicking}
          anchor={anchor}
          label={t.title}
          value={{ dueOn: t.dueOn, dueTime: t.dueTime, recurrence: t.recurrence }}
          now={now}
          onChange={onPatch}
          onCustomRepeat={onRepeat}
        />
      )}
    </li>
  );
}

function Meta({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <span title={title} aria-label={title} className="inline-flex items-center gap-1">
      {icon}
      {children}
    </span>
  );
}
