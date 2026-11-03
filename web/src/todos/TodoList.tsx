// The user's todo list (PDT-125): the right panel on desktop and the phone's Todos sheet. Due dates and their groups
// (PDT-140); repeating todos (PDT-141); the redesign (PDT-143): a header with the scope, quick add, rows, six groups,
// Move to today, the due picker, and ticked rows that linger before moving to Completed. A todo opens large in place of
// the list (PDT-227).
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { CalendarDays, ChevronDown, ChevronRight, CircleAlert, CircleArrowRight, Inbox, Sun, Sunrise } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Alert, Button, Checkbox, Dialog, EmptyState, Field, Input, MenuButton, SegmentedControl, Select, Skeleton } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Recurrence } from '../api/gen/Recurrence';
import type { RepeatFrom } from '../api/gen/RepeatFrom';
import type { RepeatRule } from '../api/gen/RepeatRule';
import type { Todo } from '../api/gen/Todo';
import { useOpenTicket } from '../common/open';
import { TicketPicker } from '../tickets/TicketPicker';
import { defaultRecurrence, grouped, localDay, moveToToday, positionFor, repeatText, shown, weekdayOf, WEEKDAYS, type DueGroupId, type TodoFilter } from './model';
import { QuickAdd } from './QuickAdd';
import { TodoRow } from './TodoRow';
import { TodoView } from './TodoView';
import { useTodoActions, useTodos } from './useTodos';

export const TODO_FILTER_KEY = 'pdt.todos.filter';
/** How long a todo opened from a notification keeps its ring. */
const HIGHLIGHT_MS = 2500;
/** How long a ticked todo stays in its place before it moves to Completed. */
export const COMPLETE_LINGER_MS = 800;

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

/** A todo to open (its open view): a notification asked for it. `seq` changes each time, so the same todo can be opened again. */
export type TodoFocus = { id: string; seq: number };

const GROUP_ICON: Record<DueGroupId, ReactNode> = {
  overdue: <CircleAlert aria-hidden />,
  today: <Sun aria-hidden className="text-waiting" />,
  tomorrow: <Sunrise aria-hidden className="text-muted-foreground" />,
  week: <CalendarDays aria-hidden className="text-muted-foreground" />,
  later: <CircleArrowRight aria-hidden className="text-muted-foreground" />,
  none: <Inbox aria-hidden className="text-muted-foreground" />,
};

/** The repeat editor, open for a todo or for quick add's new one. */
type Repeating = { title: string; initial: Recurrence; dueOn: string | null; save: (r: Recurrence, dueOn: string | null) => void };

/**
 * The header ("Todos", the open count, the scope: All projects or the one on screen), quick add, then the open todos
 * grouped as Overdue, Today, Tomorrow, Next 7 days, Later and No date (headings once something has a date), and the
 * done ones folded into "Completed · n". Rows check off, rename in place, take a due date from the picker, drag (or
 * Alt+↑/↓) within their group, and link to a project or ticket from ⋯. A click on a title opens the todo in place of the
 * list (its notes, and the same controls), with Back to the list.
 */
export function TodoList({ projectId, projects, focus, onFocused, focusAddSeq, trailing, onOpenChange }: {
  /** The project on screen: the scope's project, and new todos' project while it is on. */
  projectId: string | null;
  projects: readonly Project[];
  /** A todo to show once. */
  focus?: TodoFocus | null;
  /** The list has handled `focus` (shown it, or found it gone): the caller drops it. */
  onFocused?: () => void;
  /** Changes when quick add should take focus (the panel was opened with Ctrl+] t). */
  focusAddSeq?: number;
  /** At the end of the header (the desktop's collapse button). */
  trailing?: ReactNode;
  /** A todo was opened (true) or the list is back (false): a phone's sheet goes full screen for it. */
  onOpenChange?: (open: boolean) => void;
}) {
  const todos = useTodos();
  const actions = useTodoActions();
  const openTicket = useOpenTicket();
  const [filter, setFilterState] = useState<TodoFilter>(readFilter);
  const [doneOpen, setDoneOpen] = useState(false);
  const [linking, setLinking] = useState<Todo | null>(null);
  const [repeating, setRepeating] = useState<Repeating | null>(null);
  const [deleting, setDeleting] = useState<Todo | null>(null);
  const [highlight, setHighlight] = useState<string | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [completing, setCompleting] = useState<ReadonlySet<string>>(new Set());
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const now = new Date();
  const project = projects.find((p) => p.id === projectId) ?? null;
  const active = filter === 'project' && project ? 'project' : 'all';
  const all = todos.data ?? [];
  const list = shown(all, active, project?.id ?? null);
  // Just ticked: still among the open ones (as they were) until the delay is up.
  const lingering = list.done.filter((t) => completing.has(t.id));
  const open = [...list.open, ...lingering].sort((a, b) => a.position - b.position);
  const done = list.done.filter((t) => !completing.has(t.id));
  const byId = new Map(open.map((t) => [t.id, t]));
  const groups = grouped(open.map((t) => (t.done ? { ...t, done: false } : t)), now)
    .map((g) => ({ ...g, todos: g.todos.map((t) => byId.get(t.id)!) }));
  // Headings only once something has a date: a list of undated todos is just a list.
  const headings = groups.some((g) => g.id !== 'none');
  const openCount = list.open.length;
  const setFilter = (f: TodoFilter) => {
    setFilterState(f);
    writeFilter(f);
  };
  const move = (group: readonly Todo[], id: string, targetId: string, where: 'before' | 'after') => {
    const position = positionFor(group, id, targetId, where);
    if (position !== null) void actions.patch(id, { position });
  };
  const tick = (t: Todo, isDone: boolean) => {
    void actions.patch(t.id, { done: isDone });
    const timer = timers.current.get(t.id);
    if (timer) clearTimeout(timer);
    timers.current.delete(t.id);
    if (isDone && !t.done) {
      setCompleting((s) => new Set([...s, t.id]));
      timers.current.set(t.id, setTimeout(() => {
        timers.current.delete(t.id);
        setCompleting((s) => new Set([...s].filter((x) => x !== t.id)));
      }, COMPLETE_LINGER_MS));
    } else if (!isDone) {
      setCompleting((s) => (s.has(t.id) ? new Set([...s].filter((x) => x !== t.id)) : s));
    }
  };
  useEffect(() => {
    const pending = timers.current;
    return () => pending.forEach(clearTimeout);
  }, []);

  const openChange = useRef(onOpenChange);
  openChange.current = onOpenChange;
  const isOpen = openId !== null;
  useEffect(() => {
    openChange.current?.(isOpen);
  }, [isOpen]);
  // Back to the list: the todo that was open keeps a ring for a moment, so the eye finds it.
  const closeTodo = () => {
    setHighlight(openId);
    setOpenId(null);
  };

  // Opened from a notification: open it (and, behind it, show it in the list: All if the filter hides it, Completed
  // unfolded if it is done), then hand the request back (`onFocused`) so a later mount of the list (the panel
  // reopened) doesn't open it again.
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
      setOpenId(focusId);
    }
    focused.current?.();
    // Once per request (the list re-renders on every poll).
  }, [focusId, focusSeq, loaded]);
  useEffect(() => {
    if (!highlight || openId) return;
    const scroll = requestAnimationFrame(() => document.getElementById(`todo-${highlight}`)?.scrollIntoView?.({ block: 'nearest' }));
    const off = setTimeout(() => setHighlight(null), HIGHLIGHT_MS);
    return () => {
      cancelAnimationFrame(scroll);
      clearTimeout(off);
    };
  }, [highlight, openId]);

  const editRepeat = (t: Todo) => setRepeating({
    title: t.title,
    initial: t.recurrence ?? defaultRecurrence(t.dueOn, now),
    dueOn: t.dueOn,
    save: (recurrence, dueOn) => void actions.patch(t.id, { recurrence, ...(dueOn ? { dueOn } : {}) }),
  });

  const row = (t: Todo, group: DueGroupId | null, list?: readonly Todo[], i = 0) => (
    <TodoRow
      key={t.id}
      todo={t}
      now={now}
      group={group}
      showProject={active === 'all'}
      projects={projects}
      highlighted={highlight === t.id}
      completing={completing.has(t.id)}
      onOpen={() => setOpenId(t.id)}
      onPatch={(p) => (p.done !== undefined ? tick(t, p.done) : void actions.patch(t.id, p))}
      onDelete={() => (!t.done && t.recurrence ? setDeleting(t) : void actions.remove([t.id]))}
      onLink={() => setLinking(t)}
      onRepeat={() => editRepeat(t)}
      onOpenTicket={openTicket}
      {...(list && !completing.has(t.id) ? {
        onMove: (where: 'before' | 'after') => {
          const target = where === 'before' ? list[i - 1] : list[i + 1];
          if (target) move(list, t.id, target.id, where);
        },
        onDropHere: (id: string, where: 'before' | 'after') => move(list, id, t.id, where),
      } : {})}
    />
  );

  const scopeLabel = active === 'project' && project ? project.key || project.name : 'All projects';
  const removeRepeating = (t: Todo, series: 'only' | 'series') => {
    setDeleting(null);
    if (t.id === openId) setOpenId(null);
    void actions.remove([t.id], series);
  };
  const opened = openId ? all.find((t) => t.id === openId) : undefined;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {openId ? (
        <TodoView
          key={openId}
          todo={opened}
          now={now}
          projects={projects}
          onBack={closeTodo}
          onPatch={(p) => { if (opened) void actions.patch(opened.id, p); }}
          onDelete={() => {
            if (!opened) return;
            if (!opened.done && opened.recurrence) setDeleting(opened);
            else {
              setOpenId(null);
              void actions.remove([opened.id]);
            }
          }}
          onLink={() => opened && setLinking(opened)}
          onRepeat={() => opened && editRepeat(opened)}
          onOpenTicket={openTicket}
        />
      ) : (
        <>
        <div className="flex shrink-0 items-center gap-2 pt-3 pr-2 pb-2 pl-4">
          <h2 className="text-sm font-semibold tracking-tight">Todos</h2>
          {openCount > 0 && <span className="text-xs text-muted-foreground tabular-nums" aria-label={`${openCount} open`}>{openCount}</span>}
          <MenuButton
            label="Scope"
            entries={() => [
              { label: `${active === 'all' ? '✓ ' : ''}All projects`, onSelect: () => setFilter('all') },
              { label: `${active === 'project' ? '✓ ' : ''}${project ? project.key || project.name : 'This project'}`, disabled: !project, onSelect: () => setFilter('project') },
            ]}
          >
            <Button variant="ghost" size="sm" aria-label={`Scope: ${scopeLabel}`} className="ml-auto gap-1 font-normal text-muted-foreground">
              {scopeLabel}
              <ChevronDown aria-hidden className="size-3.5" />
            </Button>
          </MenuButton>
          {trailing}
        </div>
        <div className="shrink-0 px-3 pb-2">
          <QuickAdd
            now={now}
            projects={projects}
            projectId={active === 'project' && project ? project.id : null}
            focusSeq={focusAddSeq}
            onAdd={(t) => actions.add(t)}
            onCustomRepeat={(title, current, set) => setRepeating({
              title,
              initial: current.recurrence ?? defaultRecurrence(current.dueOn, now),
              dueOn: current.dueOn,
              save: (recurrence, dueOn) => set({ ...current, recurrence, dueOn: current.dueOn ?? dueOn }),
            })}
          />
        </div>
        <div className="flex min-h-0 flex-1 flex-col overflow-auto px-1.5 pb-3">
          {todos.isPending && <Skeleton lines={4} label="Loading todos" />}
          {todos.isError && <Alert tone="error">Todos could not be loaded: {todos.error.message}</Alert>}
          {todos.isSuccess && open.length === 0 && done.length === 0 && (
            <EmptyState>{active === 'project' ? `No todos for ${project?.key ?? 'this project'} yet.` : 'Nothing to do yet.'}</EmptyState>
          )}
          {groups.map((g) => (
            <section key={g.id} aria-label={g.label} className="mt-2.5 flex flex-col first:mt-1">
              {headings && (
                <h3 className={cn('flex min-h-6.5 items-center gap-1.5 px-2.5 text-xs font-semibold [&>svg]:size-3.5', g.id === 'overdue' && 'text-destructive')}>
                  {GROUP_ICON[g.id]}
                  {g.label}
                  <span className="font-medium text-muted-foreground tabular-nums">{g.todos.length}</span>
                  {g.id === 'overdue' && (
                    <Button
                      variant="secondary"
                      size="sm"
                      className="ml-auto font-medium text-muted-foreground"
                      onClick={() => g.todos.forEach((t) => void actions.patch(t.id, moveToToday(t, new Date())))}
                    >
                      Move to today
                    </Button>
                  )}
                </h3>
              )}
              <ul aria-label={headings ? `${g.label} todos` : 'Todos'} className="flex flex-col">
                {g.todos.map((t, i) => row(t, g.id, g.todos, i))}
              </ul>
            </section>
          ))}
          {done.length > 0 && (
            <section aria-label="Completed" className="mt-3.5 flex flex-col">
              <div className="flex items-center gap-1 px-1">
                <Button variant="ghost" size="sm" aria-expanded={doneOpen} onClick={() => setDoneOpen((o) => !o)} className="font-medium text-muted-foreground">
                  {doneOpen ? <ChevronDown aria-hidden className="size-3.5" /> : <ChevronRight aria-hidden className="size-3.5" />}
                  Completed · {done.length}
                </Button>
                <Button variant="ghost" size="sm" className="ml-auto font-normal text-muted-foreground" onClick={() => void actions.remove(done.map((t) => t.id))}>
                  Clear
                </Button>
              </div>
              {doneOpen && (
                <ul aria-label="Completed todos" className="flex flex-col">
                  {done.map((t) => row(t, null))}
                </ul>
              )}
            </section>
          )}
        </div>
        </>
      )}
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
      {repeating && (
        <RepeatDialog
          title={repeating.title}
          initial={repeating.initial}
          dueOn={repeating.dueOn}
          now={now}
          onClose={() => setRepeating(null)}
          onSave={(recurrence, dueOn) => {
            setRepeating(null);
            repeating.save(recurrence, dueOn);
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
              <Button onClick={() => removeRepeating(deleting, 'only')}>Only this one</Button>
              <Button variant="danger" onClick={() => removeRepeating(deleting, 'series')}>Stop the series</Button>
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
