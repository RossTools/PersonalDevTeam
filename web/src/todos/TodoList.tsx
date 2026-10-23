// The user's todo list (PDT-125): the right panel's Todos tab on desktop and the phone's checklist sheet.
import { useState, type DragEvent, type KeyboardEvent } from 'react';
import { ChevronDown, ChevronRight, GripVertical } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Alert, Button, Checkbox, Dialog, dragData, EmptyState, Input, MenuButton, parseDragData, SegmentedControl, Skeleton, type MenuEntry } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import { ProjectKey } from '../common/BranchChip';
import { InlineRename } from '../common/InlineRename';
import { TicketKey } from '../common/TicketKey';
import { useOpenTicket } from '../tickets/actions';
import { TicketPicker } from '../tickets/TicketPicker';
import { positionFor, shown, type TodoFilter } from './model';
import { useTodoActions, useTodos } from './useTodos';

const DRAG_KIND = 'todo';
export const TODO_FILTER_KEY = 'pdt.todos.filter';

function readFilter(): TodoFilter {
  try {
    return localStorage.getItem(TODO_FILTER_KEY) === 'project' ? 'project' : 'all';
  } catch {
    return 'all';
  }
}

function writeFilter(f: TodoFilter) {
  try {
    localStorage.setItem(TODO_FILTER_KEY, f);
  } catch {
    // Storage blocked: the filter lasts for this visit.
  }
}

/**
 * Add a todo at the top, filter by All or the current project, check off, edit in place, drag (or Alt+↑/↓) to
 * reorder, link to a project or ticket from ⋯. Done todos fold into "Done (n)".
 */
export function TodoList({ projectId, projects }: {
  /** The project on screen: the filter's "current project", and new todos' project while it is on. */
  projectId: string | null;
  projects: readonly Project[];
}) {
  const todos = useTodos();
  const actions = useTodoActions();
  const [filter, setFilterState] = useState<TodoFilter>(readFilter);
  const [doneOpen, setDoneOpen] = useState(false);
  const [linking, setLinking] = useState<Todo | null>(null);
  const project = projects.find((p) => p.id === projectId) ?? null;
  const active = filter === 'project' && project ? 'project' : 'all';
  const { open, done } = shown(todos.data ?? [], active, project?.id ?? null);
  const setFilter = (f: TodoFilter) => {
    setFilterState(f);
    writeFilter(f);
  };
  const move = (id: string, targetId: string, where: 'before' | 'after') => {
    const position = positionFor(open, id, targetId, where);
    if (position !== null) void actions.patch(id, { position });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2">
      <AddTodo onAdd={(title) => actions.add({ title, ...(active === 'project' && project ? { project: project.id } : {}) })} />
      <SegmentedControl
        aria-label="Show"
        size="sm"
        value={active}
        onChange={setFilter}
        options={[
          { value: 'all', label: 'All' },
          { value: 'project', label: project?.key || 'This project', disabled: !project },
        ]}
      />
      <div className="-mx-2 flex min-h-0 flex-1 flex-col overflow-auto px-2">
        {todos.isPending && <Skeleton lines={4} label="Loading todos" />}
        {todos.isError && <Alert tone="error">Todos could not be loaded: {todos.error.message}</Alert>}
        {todos.isSuccess && open.length === 0 && done.length === 0 && (
          <EmptyState>{active === 'project' ? `No todos for ${project?.key ?? 'this project'} yet.` : 'Nothing to do yet.'}</EmptyState>
        )}
        {open.length > 0 && (
          <ul aria-label="Todos" className="flex flex-col gap-0.5">
            {open.map((t, i) => (
              <TodoRow
                key={t.id}
                todo={t}
                showProject={active === 'all'}
                projects={projects}
                onLink={() => setLinking(t)}
                onMove={(where) => {
                  const target = where === 'before' ? open[i - 1] : open[i + 1];
                  if (target) move(t.id, target.id, where);
                }}
                onDropHere={(id, where) => move(id, t.id, where)}
              />
            ))}
          </ul>
        )}
        {done.length > 0 && (
          <section aria-label="Done" className="mt-2 flex flex-col gap-0.5">
            <div className="flex items-center gap-1">
              <Button variant="ghost" size="sm" aria-expanded={doneOpen} onClick={() => setDoneOpen((o) => !o)} className="text-muted-foreground">
                {doneOpen ? <ChevronDown aria-hidden className="size-4" /> : <ChevronRight aria-hidden className="size-4" />}
                Done ({done.length})
              </Button>
              <Button variant="ghost" size="sm" className="ml-auto text-muted-foreground" onClick={() => void actions.remove(done.map((t) => t.id))}>
                Clear done
              </Button>
            </div>
            {doneOpen && (
              <ul aria-label="Done todos" className="flex flex-col gap-0.5">
                {done.map((t) => (
                  <TodoRow key={t.id} todo={t} showProject={active === 'all'} projects={projects} onLink={() => setLinking(t)} />
                ))}
              </ul>
            )}
          </section>
        )}
      </div>
      {linking && (
        <Dialog open onOpenChange={(o) => !o && setLinking(null)} title="Link to a ticket" description={linking.title}>
          <TicketPicker
            projectId={null}
            label="Ticket to link"
            onPick={(k) => {
              setLinking(null);
              void actions.patch(linking.id, { ticket: k.id });
            }}
            className="max-h-[50dvh]"
          />
        </Dialog>
      )}
    </div>
  );
}

function AddTodo({ onAdd }: { onAdd: (title: string) => Promise<unknown> }) {
  const [title, setTitle] = useState('');
  const [pending, setPending] = useState(false);
  const add = async () => {
    const t = title.trim();
    if (!t || pending) return;
    setPending(true);
    const made = await onAdd(t);
    setPending(false);
    if (made) setTitle('');
  };
  return (
    <Input
      aria-label="Add a todo"
      placeholder="Add a todo…"
      value={title}
      size="sm"
      maxLength={500}
      disabled={pending}
      onChange={(e) => setTitle(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
          e.preventDefault();
          void add();
        }
      }}
    />
  );
}

function TodoRow({ todo: t, showProject, projects, onLink, onMove, onDropHere }: {
  todo: Todo;
  showProject: boolean;
  projects: readonly Project[];
  onLink: () => void;
  /** Open rows: Alt+↑/↓ moves the row past its neighbour. */
  onMove?: (where: 'before' | 'after') => void;
  /** Open rows: another row was dropped on this one's top or bottom half. */
  onDropHere?: (id: string, where: 'before' | 'after') => void;
}) {
  const actions = useTodoActions();
  const openTicket = useOpenTicket();
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
    { label: t.ticketId ? 'Link to another ticket…' : 'Link to a ticket…', onSelect: onLink },
    ...(t.ticketId ? [{ label: `Unlink ${t.ticketKey ?? 'the ticket'}`, onSelect: () => void actions.patch(t.id, { ticket: '' }) }] : []),
    {
      submenu: 'Project',
      entries: [
        { label: t.projectId ? 'No project' : '✓ No project', onSelect: () => void actions.patch(t.id, { project: '' }) },
        ...projects.map((p) => ({ label: p.id === t.projectId ? `✓ ${p.key || p.name}` : p.key || p.name, onSelect: () => void actions.patch(t.id, { project: p.id }) })),
      ],
    },
    { separator: true },
    { label: 'Delete', danger: true, onSelect: () => void actions.remove([t.id]) },
  ];
  return (
    <li
      aria-label={t.title}
      draggable={draggable}
      onDragStart={draggable ? (e) => {
        e.dataTransfer.setData('text/plain', dragData(DRAG_KIND, t.id));
        e.dataTransfer.effectAllowed = 'move';
      } : undefined}
      onDragOver={onDropHere ? (e) => { e.preventDefault(); setOver(half(e)); } : undefined}
      onDragLeave={() => setOver(null)}
      onDrop={onDropHere ? (e) => {
        setOver(null);
        const d = parseDragData(e.dataTransfer.getData('text/plain') ?? '');
        if (!d || d.kind !== DRAG_KIND || d.id === t.id) return;
        e.preventDefault();
        onDropHere(d.id, half(e));
      } : undefined}
      onKeyDown={onKeyDown}
      className={cn(
        'group/todo flex min-w-0 items-start gap-1.5 rounded-md border-y-2 border-transparent px-1 py-0.5 text-sm hover:bg-muted',
        over === 'before' && 'border-t-primary',
        over === 'after' && 'border-b-primary',
      )}
    >
      {onDropHere ? (
        <GripVertical aria-hidden className="mt-1 size-3 shrink-0 cursor-grab text-muted-foreground opacity-0 group-hover/todo:opacity-100 pointer-coarse:hidden" />
      ) : <span aria-hidden className="w-3 shrink-0" />}
      <Checkbox
        aria-label={t.done ? `Not done: ${t.title}` : `Done: ${t.title}`}
        checked={t.done}
        onChange={(done) => void actions.patch(t.id, { done })}
        className="mt-0.5"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {editing ? (
          <InlineRename
            label="Todo"
            initial={t.title}
            maxLength={500}
            onCommit={(title) => { setEditing(false); void actions.patch(t.id, { title }); }}
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
        {((showProject && t.projectKey) || t.ticketKey) && (
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            {showProject && t.projectKey && !t.ticketKey && <ProjectKey projectKey={t.projectKey} />}
            {t.ticketKey && t.ticketId && t.projectId && (
              <TicketKey
                ticketKey={t.ticketKey}
                title={t.ticketTitle ?? undefined}
                onClick={() => openTicket({ id: t.ticketId!, projectId: t.projectId! })}
              />
            )}
          </div>
        )}
      </div>
      <MenuButton label={`Actions for ${t.title}`} size="sm" entries={entries} className="shrink-0 opacity-0 group-hover/todo:opacity-100 focus-visible:opacity-100 data-pressed:opacity-100 pointer-coarse:opacity-100" />
    </li>
  );
}
