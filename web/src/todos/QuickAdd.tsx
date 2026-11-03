// Quick add (PDT-143): the todo list's add box. Dates, times and repeats typed into the title ("tomorrow 3pm every
// month") are picked out as you type (`parseTodo`), highlighted, and shown as tags under the text; × on a tag keeps its
// words in the title. Dashed tags set what wasn't typed: Date and Repeat (the due picker), Project, Ticket. Enter adds.
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { CalendarDays, CornerDownLeft, Hash, Plus, Repeat, Sun, Sunrise, Ticket, X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Dialog, MenuButton } from '@ui';
import type { NewTodo } from '../api/gen/NewTodo';
import type { Project } from '../api/gen/Project';
import { useAction } from '../common/useAction';
import { TicketPicker } from '../tickets/TicketPicker';
import { dayText, DuePicker, type DuePanel, type DueValue } from './DuePicker';
import { addDays, localDay, repeatText } from './model';
import { parseTodo, type ParseKind } from './parse';
import { TicketKey } from '../common/TicketKey';

const NONE: DueValue = { dueOn: null, dueTime: null, recurrence: null };

export function QuickAdd({ now, projects, projectId, focusSeq, onAdd, onCustomRepeat }: {
  now: Date;
  projects: readonly Project[];
  /** The scope's project: new todos join it unless another is chosen. */
  projectId: string | null;
  /** Changes when the box should take focus (the panel was opened with Ctrl+] t). */
  focusSeq?: number;
  /** Adds the todo; a falsy result (it failed) keeps the text. */
  onAdd: (t: NewTodo) => Promise<unknown>;
  /** Custom… in the picker's Repeat panel: the full editor, whose rule comes back through the returned setter. */
  onCustomRepeat: (title: string, current: DueValue, set: (v: DueValue) => void) => void;
}) {
  const [text, setText] = useState('');
  // × on a tag (or a pick by hand) dismisses a kind's typed phrases; it holds while those words are still in the text.
  const [dismissed, setDismissed] = useState<Partial<Record<ParseKind, string[]>>>({});
  const [manual, setManual] = useState<DueValue>(NONE);
  const [project, setProject] = useState<string | null>(null);
  const [ticket, setTicket] = useState<{ id: string; key: string } | null>(null);
  const [picker, setPicker] = useState<DuePanel | null>(null);
  const [linking, setLinking] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [focused, setFocused] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  const anchor = useRef<Element | null>(null);

  useEffect(() => {
    if (focusSeq) input.current?.focus();
  }, [focusSeq]);

  const ignore = ignored(dismissed, text);
  const parsed = parseTodo(text, now, ignore);
  const typedWhen = parsed.matches.some((m) => m.kind === 'when');
  const typedRepeat = parsed.matches.some((m) => m.kind === 'repeat');
  const due: DueValue = {
    dueOn: parsed.dueOn ?? manual.dueOn,
    dueTime: parsed.dueOn ? parsed.dueTime ?? null : manual.dueTime,
    recurrence: parsed.recurrence ?? manual.recurrence,
  };
  const chosenProject = project ?? projectId;
  const projectKey = chosenProject ? projects.find((p) => p.id === chosenProject)?.key ?? null : null;
  const expanded = focused || !!text || picker !== null || linking || menuOpen;

  const reset = () => {
    setText('');
    setDismissed({});
    setManual(NONE);
    setProject(null);
    setTicket(null);
  };
  const [submit, pending] = useAction(async () => {
    const dueOn = due.dueOn ?? (due.recurrence ? localDay(now) : null);
    const made = await onAdd({
      title: parsed.title,
      ...(chosenProject ? { project: chosenProject } : {}),
      ...(ticket ? { ticket: ticket.id } : {}),
      ...(dueOn ? { dueOn } : {}),
      ...(dueOn && due.dueTime ? { dueTime: due.dueTime } : {}),
      ...(due.recurrence ? { recurrence: due.recurrence } : {}),
    });
    if (made) reset();
    input.current?.focus();
  });
  const add = async () => {
    if (text.trim() && !pending) await submit();
  };
  const openPicker = (panel: DuePanel, from: Element | null) => {
    anchor.current = from;
    setPicker(panel);
  };
  const dismiss = (kind: ParseKind) => {
    const words = parsed.matches.filter((m) => m.kind === kind).map((m) => m.text.toLowerCase());
    if (words.length) setDismissed((d) => ({ ...d, [kind]: words }));
  };
  /** A tag's ×: clear it, then back to the text. */
  const clearing = (fn: () => void) => () => {
    fn();
    input.current?.focus();
  };
  const whenIcon = due.dueOn === localDay(now)
    ? <Sun aria-hidden className="text-waiting" />
    : due.dueOn === addDays(localDay(now), 1) ? <Sunrise aria-hidden className="text-waiting" /> : <CalendarDays aria-hidden />;

  return (
    <div
      ref={box}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        if (!box.current?.contains(e.relatedTarget as Node | null)) setFocused(false);
      }}
      className={cn(
        'flex flex-col rounded-lg border bg-background transition-colors duration-(--pdt-duration) motion-reduce:transition-none',
        expanded ? 'border-primary ring-3 ring-ring/10' : 'border-border',
      )}
    >
      {/* A click anywhere on the line (the + circle too) puts the cursor in the text. */}
      <div
        onMouseDown={(e) => {
          if (e.target === input.current) return;
          e.preventDefault();
          input.current?.focus();
        }}
        className="flex min-h-9.5 cursor-text items-center gap-2.5 px-3"
      >
        {expanded ? (
          <span aria-hidden className="size-4.5 shrink-0 rounded-full border-[1.5px] border-muted-foreground" />
        ) : (
          <span aria-hidden className="grid size-4.5 shrink-0 place-items-center rounded-full border-[1.5px] border-dashed border-muted-foreground/60 text-link">
            <Plus className="size-3" />
          </span>
        )}
        <div className="relative min-w-0 flex-1">
          {/* The highlight: the same text, transparent, with the matched phrases filled; the input sits on top. */}
          <div ref={mirror} aria-hidden className="pointer-events-none absolute inset-0 overflow-hidden text-sm leading-9 whitespace-pre text-transparent">
            {highlight(text, parsed.matches)}
          </div>
          <input
            ref={input}
            aria-label="Add a todo"
            placeholder={expanded ? 'What needs doing?' : 'Add a todo'}
            value={text}
            maxLength={500}
            disabled={pending}
            onChange={(e) => {
              setText(e.target.value);
            }}
            onScroll={(e) => {
              if (mirror.current) mirror.current.scrollLeft = e.currentTarget.scrollLeft;
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void add();
              } else if (e.key === 'Escape' && text) {
                e.preventDefault();
                reset();
              }
            }}
            className="relative h-9 w-full bg-transparent text-sm leading-9 text-foreground outline-none placeholder:text-muted-foreground"
          />
        </div>
      </div>
      {expanded && (
        <div className="flex flex-wrap items-center gap-1.5 pr-2.5 pb-2.5 pl-10">
          {due.dueOn ? (
            <Tag
              icon={whenIcon}
              clearLabel={typedWhen ? 'Keep the date words in the title' : 'No date'}
              onClear={clearing(() => (typedWhen ? dismiss('when') : setManual((m) => ({ ...m, dueOn: null, dueTime: null }))))}
            >
              {dayText(due.dueOn, now)}{due.dueTime ? ` ${due.dueTime}` : ''}
            </Tag>
          ) : (
            <DashedTag icon={<CalendarDays aria-hidden />} onClick={(el) => openPicker('date', el)}>Date</DashedTag>
          )}
          {due.recurrence ? (
            <Tag
              icon={<Repeat aria-hidden />}
              clearLabel={typedRepeat ? 'Keep the repeat words in the title' : 'Doesn’t repeat'}
              onClear={clearing(() => (typedRepeat ? dismiss('repeat') : setManual((m) => ({ ...m, recurrence: null }))))}
            >
              {repeatText(due.recurrence)}
            </Tag>
          ) : (
            <DashedTag icon={<Repeat aria-hidden />} onClick={(el) => openPicker('repeat', el)}>Repeat</DashedTag>
          )}
          {projectKey && project ? (
            <Tag icon={<Hash aria-hidden />} clearLabel="No project" onClear={clearing(() => setProject(null))}>{projectKey}</Tag>
          ) : (
            <MenuButton
              label="Project for the new todo"
              placement="bottom start"
              onOpenChange={setMenuOpen}
              entries={() => projects.map((p) => ({ label: p.id === chosenProject ? `✓ ${p.key || p.name}` : p.key || p.name, onSelect: () => setProject(p.id) }))}
            >
              <button type="button" className={dashed}>
                <Hash aria-hidden />
                {projectKey ?? 'Project'}
              </button>
            </MenuButton>
          )}
          {ticket ? (
            <Tag icon={<Ticket aria-hidden />} clearLabel="No ticket" onClear={clearing(() => setTicket(null))}><TicketKey ticketKey={ticket.key} /></Tag>
          ) : (
            <DashedTag icon={<Ticket aria-hidden />} onClick={() => setLinking(true)}>Ticket</DashedTag>
          )}
          <span className="ml-auto inline-flex items-center gap-1 text-meta text-muted-foreground pointer-coarse:hidden">
            <kbd className="inline-flex h-4 items-center rounded-sm border border-border px-1 font-sans"><CornerDownLeft aria-hidden className="size-2.5" /></kbd>
            add
          </span>
        </div>
      )}
      {picker && (
        <DuePicker
          open
          onOpenChange={(o) => {
            if (!o) {
              setPicker(null);
              input.current?.focus();
            }
          }}
          anchor={anchor}
          panel={picker}
          label={parsed.title || 'New todo'}
          value={due}
          now={now}
          onChange={(p) => {
            // Picked by hand: it replaces what was typed, whose words then stay in the title.
            if ((p.dueOn !== undefined || p.dueTime !== undefined) && typedWhen) dismiss('when');
            if (p.recurrence !== undefined && typedRepeat) dismiss('repeat');
            setManual((m) => {
              const base = { dueOn: due.dueOn, dueTime: due.dueTime, recurrence: due.recurrence ?? m.recurrence };
              const cleared = p.dueOn === '';
              return {
                dueOn: p.dueOn !== undefined ? p.dueOn || null : base.dueOn,
                dueTime: cleared ? null : p.dueTime !== undefined ? p.dueTime || null : base.dueTime,
                recurrence: cleared ? null : p.recurrence !== undefined ? p.recurrence : base.recurrence,
              };
            });
          }}
          onCustomRepeat={() => onCustomRepeat(parsed.title || 'New todo', due, (v) => {
            if (typedRepeat) dismiss('repeat');
            setManual(v);
          })}
        />
      )}
      {linking && (
        <Dialog onClose={() => setLinking(false)} title="Link to a ticket" description={parsed.title || 'New todo'}>
          <TicketPicker
            projectId={null}
            label="Ticket to link"
            onPick={(k) => {
              setLinking(false);
              setTicket({ id: k.id, key: k.key });
              input.current?.focus();
            }}
            className="max-h-[50dvh]"
          />
        </Dialog>
      )}
    </div>
  );
}

/** The kinds whose dismissed phrases are all still in the text (once one is edited away, its kind matches again). */
function ignored(dismissed: Partial<Record<ParseKind, string[]>>, text: string): ReadonlySet<ParseKind> {
  const lower = text.toLowerCase();
  return new Set((Object.keys(dismissed) as ParseKind[]).filter((k) => dismissed[k]!.every((w) => lower.includes(w))));
}

/** The text with its matched phrases wrapped in a soft primary fill. */
function highlight(text: string, matches: readonly { start: number; end: number }[]): ReactNode[] {
  const out: ReactNode[] = [];
  let at = 0;
  for (const m of [...matches].sort((a, b) => a.start - b.start)) {
    if (m.start < at) continue;
    out.push(text.slice(at, m.start));
    out.push(<mark key={m.start} className="rounded-sm bg-primary/25 text-transparent">{text.slice(m.start, m.end)}</mark>);
    at = m.end;
  }
  out.push(text.slice(at));
  return out;
}

const dashed =
  'inline-flex h-6 items-center gap-1.5 rounded-md border border-dashed border-border px-2 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch [&>svg]:size-3';

function DashedTag({ icon, onClick, children }: { icon: ReactNode; onClick: (el: HTMLElement) => void; children: ReactNode }) {
  return (
    <button type="button" className={dashed} onClick={(e) => onClick(e.currentTarget)}>
      {icon}
      {children}
    </button>
  );
}

function Tag({ icon, clearLabel, onClear, children }: { icon: ReactNode; clearLabel: string; onClear: () => void; children: ReactNode }) {
  return (
    <span className="inline-flex h-6 items-center gap-1.5 rounded-md bg-surface-2 pr-0.5 pl-2 text-xs pointer-coarse:min-h-control-touch [&>svg]:size-3">
      {icon}
      {children}
      <button
        type="button"
        aria-label={clearLabel}
        title={clearLabel}
        onClick={onClear}
        className="grid size-5 place-items-center rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:size-control-touch"
      >
        <X aria-hidden className="size-3" />
      </button>
    </span>
  );
}
