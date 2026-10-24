// The user's todo list (PDT-125): the right panel's Todos tab on desktop and the phone's checklist sheet. Cards, due
// dates and their groups (PDT-140).
import { useEffect, useState } from 'react';
import { CalendarPlus, ChevronDown, ChevronRight, X } from 'lucide-react';
import { Alert, Button, Dialog, EmptyState, Field, IconButton, Input, MenuButton, SegmentedControl, Skeleton } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import { useOpenTicket } from '../tickets/actions';
import { TicketPicker } from '../tickets/TicketPicker';
import { formatDay, grouped, localDay, positionFor, shown, type TodoFilter } from './model';
import { dueEntries, TodoCard } from './TodoCard';
import { useTodoActions, useTodos } from './useTodos';

export const TODO_FILTER_KEY = 'pdt.todos.filter';
/** How long a todo opened from a notification keeps its ring. */
const HIGHLIGHT_MS = 2500;

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

/** A due date and time as the API takes them. */
type Due = { dueOn: string; dueTime: string };

/** A todo to open: a notification asked for it. `seq` changes each time, so the same todo can be opened again. */
export type TodoFocus = { id: string; seq: number };

/**
 * Add a todo at the top (with a due date if you like), filter by All or the current project, check off, edit in
 * place, set a due date, drag (or Alt+↑/↓) to reorder within a group, link to a project or ticket from ⋯. Open todos
 * group as Overdue, Today, Upcoming and No date; done ones fold into "Done (n)".
 */
export function TodoList({ projectId, projects, focus }: {
  /** The project on screen: the filter's "current project", and new todos' project while it is on. */
  projectId: string | null;
  projects: readonly Project[];
  focus?: TodoFocus | null;
}) {
  const todos = useTodos();
  const actions = useTodoActions();
  const openTicket = useOpenTicket();
  const [filter, setFilterState] = useState<TodoFilter>(readFilter);
  const [doneOpen, setDoneOpen] = useState(false);
  const [linking, setLinking] = useState<Todo | null>(null);
  const [dating, setDating] = useState<Todo | null>(null);
  const [highlight, setHighlight] = useState<string | null>(null);
  const now = new Date();
  const project = projects.find((p) => p.id === projectId) ?? null;
  const active = filter === 'project' && project ? 'project' : 'all';
  const { open, done } = shown(todos.data ?? [], active, project?.id ?? null);
  const groups = grouped(open, now);
  // Headings only once something has a date: a list of undated todos is just a list.
  const headings = groups.some((g) => g.id !== 'none');
  const setFilter = (f: TodoFilter) => {
    setFilterState(f);
    writeFilter(f);
  };
  const move = (group: readonly Todo[], id: string, targetId: string, where: 'before' | 'after') => {
    const position = positionFor(group, id, targetId, where);
    if (position !== null) void actions.patch(id, { position });
  };

  // Opened from a notification: show it (All if the filter hides it, Done unfolded if it is done) with a ring.
  const focusId = focus?.id ?? null;
  const focusSeq = focus?.seq ?? 0;
  const loaded = todos.isSuccess;
  useEffect(() => {
    if (!focusId || !loaded) return;
    const t = todos.data?.find((x) => x.id === focusId);
    if (!t) return;
    if (active === 'project' && t.projectId !== project?.id) setFilterState('all');
    if (t.done) setDoneOpen(true);
    setHighlight(focusId);
    const scroll = requestAnimationFrame(() => document.getElementById(`todo-${focusId}`)?.scrollIntoView?.({ block: 'nearest' }));
    const off = setTimeout(() => setHighlight(null), HIGHLIGHT_MS);
    return () => {
      cancelAnimationFrame(scroll);
      clearTimeout(off);
    };
    // Once per request (the list re-renders on every poll).
  }, [focusId, focusSeq, loaded]);

  const card = (t: Todo, group?: readonly Todo[], i = 0) => (
    <TodoCard
      key={t.id}
      todo={t}
      now={now}
      showProject={active === 'all'}
      projects={projects}
      highlighted={highlight === t.id}
      onPatch={(p) => void actions.patch(t.id, p)}
      onDelete={() => void actions.remove([t.id])}
      onLink={() => setLinking(t)}
      onPickDate={() => setDating(t)}
      onOpenTicket={openTicket}
      {...(group ? {
        onMove: (where: 'before' | 'after') => {
          const target = where === 'before' ? group[i - 1] : group[i + 1];
          if (target) move(group, t.id, target.id, where);
        },
        onDropHere: (id: string, where: 'before' | 'after') => move(group, id, t.id, where),
      } : {})}
    />
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2">
      <AddTodo
        now={now}
        onAdd={(title, due) =>
          actions.add({
            title,
            ...(active === 'project' && project ? { project: project.id } : {}),
            ...(due ? { dueOn: due.dueOn, ...(due.dueTime ? { dueTime: due.dueTime } : {}) } : {}),
          })}
      />
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
        {groups.map((g) => (
          <section key={g.id} aria-label={g.label} className="mb-2 flex flex-col gap-1">
            {headings && (
              <h3 className={g.id === 'overdue' ? 'px-1 text-2xs font-medium tracking-wide text-destructive uppercase' : 'px-1 text-2xs font-medium tracking-wide text-muted-foreground uppercase'}>
                {g.label} <span className="tabular-nums">({g.todos.length})</span>
              </h3>
            )}
            <ul aria-label={headings ? `${g.label} todos` : 'Todos'} className="flex flex-col gap-1">
              {g.todos.map((t, i) => card(t, g.todos, i))}
            </ul>
          </section>
        ))}
        {done.length > 0 && (
          <section aria-label="Done" className="flex flex-col gap-1">
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
              <ul aria-label="Done todos" className="flex flex-col gap-1">
                {done.map((t) => card(t))}
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
      {dating && (
        <DueDialog
          title={dating.title}
          initial={{ dueOn: dating.dueOn ?? localDay(now), dueTime: dating.dueTime ?? '' }}
          onClose={() => setDating(null)}
          onSave={(due) => {
            setDating(null);
            void actions.patch(dating.id, due);
          }}
        />
      )}
    </div>
  );
}

/** Pick a date (and, if you like, a time). */
export function DueDialog({ title, initial, onSave, onClose }: {
  title: string;
  initial: Due;
  onSave: (due: Due) => void;
  onClose: () => void;
}) {
  const [dueOn, setDueOn] = useState(initial.dueOn);
  const [dueTime, setDueTime] = useState(initial.dueTime);
  const valid = /^\d{4}-\d{2}-\d{2}$/.test(dueOn);
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title="Due date"
      description={title}
      size="sm"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button disabled={!valid} onClick={() => onSave({ dueOn, dueTime })}>Save</Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (valid) onSave({ dueOn, dueTime });
        }}
      >
        <Field label="Date">
          <Input type="date" value={dueOn} required onChange={(e) => setDueOn(e.target.value)} />
        </Field>
        <Field label="Time" hint="Optional: the reminder comes then instead of at the day's start.">
          <Input type="time" value={dueTime} onChange={(e) => setDueTime(e.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

function AddTodo({ now, onAdd }: { now: Date; onAdd: (title: string, due: Due | null) => Promise<unknown> }) {
  const [title, setTitle] = useState('');
  const [due, setDue] = useState<Due | null>(null);
  const [picking, setPicking] = useState(false);
  const [pending, setPending] = useState(false);
  const add = async () => {
    const t = title.trim();
    if (!t || pending) return;
    setPending(true);
    const made = await onAdd(t, due);
    setPending(false);
    if (made) {
      setTitle('');
      setDue(null);
    }
  };
  const dueText = due && `Due ${formatDay(due.dueOn, now)}${due.dueTime ? ` ${due.dueTime}` : ''}`;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1">
        <Input
          aria-label="Add a todo"
          placeholder="Add a todo…"
          value={title}
          size="sm"
          maxLength={500}
          disabled={pending}
          className="min-w-0 flex-1"
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void add();
            }
          }}
        />
        <MenuButton
          label="Due date for the new todo"
          entries={() => dueEntries(due ?? { dueOn: null }, now, (dueOn) => setDue(dueOn ? { dueOn, dueTime: '' } : null), () => setPicking(true))}
        >
          <IconButton label="Due date for the new todo" size="sm" variant="ghost" className="text-muted-foreground">
            <CalendarPlus aria-hidden />
          </IconButton>
        </MenuButton>
      </div>
      {due && dueText && (
        <div className="flex items-center gap-1 px-1 text-meta text-muted-foreground">
          <span>{dueText}</span>
          <IconButton label="No due date" size="sm" variant="ghost" noTooltip onClick={() => setDue(null)} className="size-5">
            <X aria-hidden />
          </IconButton>
        </div>
      )}
      {picking && (
        <DueDialog
          title={title.trim() || 'New todo'}
          initial={due ?? { dueOn: localDay(now), dueTime: '' }}
          onClose={() => setPicking(false)}
          onSave={(d) => {
            setPicking(false);
            setDue(d);
          }}
        />
      )}
    </div>
  );
}
