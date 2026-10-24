// The user's todo list (PDT-125): the right panel's Todos tab on desktop and the phone's checklist sheet. Cards, due
// dates and their groups (PDT-140); repeating todos (PDT-141).
import { useEffect, useRef, useState } from 'react';
import { CalendarPlus, ChevronDown, ChevronRight, X } from 'lucide-react';
import { Alert, Button, Checkbox, Dialog, EmptyState, Field, IconButton, Input, MenuButton, SegmentedControl, Select, Skeleton } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Recurrence } from '../api/gen/Recurrence';
import type { RepeatFrom } from '../api/gen/RepeatFrom';
import type { RepeatRule } from '../api/gen/RepeatRule';
import type { Todo } from '../api/gen/Todo';
import { useOpenTicket } from '../tickets/actions';
import { TicketPicker } from '../tickets/TicketPicker';
import { defaultRecurrence, formatDay, grouped, localDay, positionFor, repeatText, shown, weekdayOf, WEEKDAYS, type TodoFilter } from './model';
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
export function TodoList({ projectId, projects, focus, onFocused }: {
  /** The project on screen: the filter's "current project", and new todos' project while it is on. */
  projectId: string | null;
  projects: readonly Project[];
  /** A todo to show once. */
  focus?: TodoFocus | null;
  /** The list has handled `focus` (shown it, or found it gone): the caller drops it. */
  onFocused?: () => void;
}) {
  const todos = useTodos();
  const actions = useTodoActions();
  const openTicket = useOpenTicket();
  const [filter, setFilterState] = useState<TodoFilter>(readFilter);
  const [doneOpen, setDoneOpen] = useState(false);
  const [linking, setLinking] = useState<Todo | null>(null);
  const [dating, setDating] = useState<Todo | null>(null);
  const [repeating, setRepeating] = useState<Todo | null>(null);
  const [deleting, setDeleting] = useState<Todo | null>(null);
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

  // Opened from a notification: show it (All if the filter hides it, Done unfolded if it is done) with a ring, then
  // hand the request back (`onFocused`) so a later mount of the list (the panel reopened) doesn't show it again.
  const focused = useRef(onFocused);
  focused.current = onFocused;
  const focusId = focus?.id ?? null;
  const focusSeq = focus?.seq ?? 0;
  const loaded = todos.isSuccess;
  useEffect(() => {
    if (!focusId || !loaded) return;
    const t = todos.data?.find((x) => x.id === focusId);
    if (t) {
      if (active === 'project' && t.projectId !== project?.id) setFilterState('all');
      if (t.done) setDoneOpen(true);
      setHighlight(focusId);
    }
    focused.current?.();
    // Once per request (the list re-renders on every poll).
  }, [focusId, focusSeq, loaded]);
  useEffect(() => {
    if (!highlight) return;
    const scroll = requestAnimationFrame(() => document.getElementById(`todo-${highlight}`)?.scrollIntoView?.({ block: 'nearest' }));
    const off = setTimeout(() => setHighlight(null), HIGHLIGHT_MS);
    return () => {
      cancelAnimationFrame(scroll);
      clearTimeout(off);
    };
  }, [highlight]);

  const card = (t: Todo, group?: readonly Todo[], i = 0) => (
    <TodoCard
      key={t.id}
      todo={t}
      now={now}
      showProject={active === 'all'}
      projects={projects}
      highlighted={highlight === t.id}
      onPatch={(p) => void actions.patch(t.id, p)}
      onDelete={() => (!t.done && t.recurrence ? setDeleting(t) : void actions.remove([t.id]))}
      onLink={() => setLinking(t)}
      onPickDate={() => setDating(t)}
      onRepeat={() => setRepeating(t)}
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
      {repeating && (
        <RepeatDialog
          title={repeating.title}
          initial={repeating.recurrence ?? defaultRecurrence(repeating.dueOn, now)}
          dueOn={repeating.dueOn}
          now={now}
          onClose={() => setRepeating(null)}
          onSave={(recurrence, dueOn) => {
            setRepeating(null);
            void actions.patch(repeating.id, { recurrence, ...(dueOn ? { dueOn } : {}) });
          }}
        />
      )}
      {deleting && (
        <Dialog
          open
          onOpenChange={(o) => !o && setDeleting(null)}
          title="Delete a repeating todo"
          description={`${deleting.title} repeats ${deleting.recurrence ? repeatText(deleting.recurrence).toLowerCase() : ''}.`}
          size="sm"
          footer={
            <>
              <Button variant="ghost" onClick={() => setDeleting(null)}>Cancel</Button>
              <Button onClick={() => { setDeleting(null); void actions.remove([deleting.id], 'only'); }}>Only this one</Button>
              <Button variant="danger" onClick={() => { setDeleting(null); void actions.remove([deleting.id], 'series'); }}>Stop the series</Button>
            </>
          }
        >
          <p className="text-sm text-muted-foreground">
            Only this one skips to the next; stopping the series deletes it and none come after it.
          </p>
        </Dialog>
      )}
    </div>
  );
}

type Every = RepeatRule['every'];

/** The Repeat… editor: a rule, what it counts from, and a first due date for a todo without one. */
export function RepeatDialog({ title, initial, dueOn: givenDueOn, now, onSave, onClose }: {
  title: string;
  initial: Recurrence;
  /** The todo's due date; without one the editor asks for it. */
  dueOn: string | null;
  now: Date;
  onSave: (r: Recurrence, dueOn: string | null) => void;
  onClose: () => void;
}) {
  const r0 = initial.rule;
  const [every, setEvery] = useState<Every>(r0.every);
  const [interval, setIntervalText] = useState(String(r0.every === 'day' || r0.every === 'week' ? r0.interval : 1));
  const [days, setDays] = useState<number[]>(r0.every === 'week' ? r0.days : [weekdayOf(givenDueOn ?? localDay(now))]);
  const [monthDay, setMonthDay] = useState(String(r0.every === 'month' ? r0.day : Number((givenDueOn ?? localDay(now)).slice(8))));
  const [from, setFrom] = useState<RepeatFrom>(initial.from);
  const [dueOn, setDueOn] = useState(givenDueOn ?? localDay(now));
  const n = Number(interval);
  const day = Number(monthDay);
  const rule: RepeatRule | null =
    every === 'day' ? (Number.isInteger(n) && n >= 1 && n <= 365 ? { every, interval: n } : null)
    : every === 'weekday' ? { every }
    : every === 'week' ? (Number.isInteger(n) && n >= 1 && n <= 365 && days.length ? { every, interval: n, days: [...days].sort((a, b) => a - b) } : null)
    : Number.isInteger(day) && day >= 1 && day <= 31 ? { every, day } : null;
  const valid = !!rule && /^\d{4}-\d{2}-\d{2}$/.test(dueOn);
  const save = () => {
    if (rule && valid) onSave({ rule, from }, givenDueOn ? null : dueOn);
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title="Repeat"
      description={title}
      size="sm"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button disabled={!valid} onClick={save}>Save</Button>
        </>
      }
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); save(); }}>
        <Field label="Repeats">
          <Select<Every>
            value={every}
            onChange={setEvery}
            options={[
              { value: 'day', label: 'Daily' },
              { value: 'weekday', label: 'Every weekday (Mon–Fri)' },
              { value: 'week', label: 'Weekly' },
              { value: 'month', label: 'Monthly' },
            ]}
          />
        </Field>
        {(every === 'day' || every === 'week') && (
          <Field label={every === 'day' ? 'Every … days' : 'Every … weeks'}>
            <Input type="number" min={1} max={365} value={interval} onChange={(e) => setIntervalText(e.target.value)} />
          </Field>
        )}
        {every === 'week' && (
          <fieldset className="flex flex-col gap-1">
            <legend className="mb-1 text-sm font-medium">On</legend>
            <div className="flex flex-wrap gap-x-3 gap-y-1">
              {WEEKDAYS.slice(1).map((name, i) => (
                <Checkbox
                  key={name}
                  label={name}
                  checked={days.includes(i + 1)}
                  onChange={(on) => setDays((d) => (on ? [...d, i + 1] : d.filter((x) => x !== i + 1)))}
                />
              ))}
            </div>
          </fieldset>
        )}
        {every === 'month' && (
          <Field label="On day" hint="A shorter month uses its last day.">
            <Input type="number" min={1} max={31} value={monthDay} onChange={(e) => setMonthDay(e.target.value)} />
          </Field>
        )}
        <fieldset className="flex flex-col gap-1">
          <legend className="mb-1 text-sm font-medium">Next one counts from</legend>
          <SegmentedControl<RepeatFrom>
            aria-label="Next one counts from"
            size="sm"
            value={from}
            onChange={setFrom}
            options={[{ value: 'due', label: 'The due date' }, { value: 'done', label: 'When it’s done' }]}
          />
        </fieldset>
        {!givenDueOn && (
          <Field label="First due" hint="A repeating todo needs a due date.">
            <Input type="date" value={dueOn} required onChange={(e) => setDueOn(e.target.value)} />
          </Field>
        )}
      </form>
    </Dialog>
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
