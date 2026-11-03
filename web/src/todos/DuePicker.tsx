// The due-date picker (PDT-143): one popover for a todo's day, time and repeat rule. Panel A types a date ("fri 9am",
// parsed like quick add), picks one quickly or from a month; its footer opens panel B (time slots) and panel C
// (repeat presets). Each change is handed out at once (`onChange`); a quick pick or a calendar day closes it.
import { useState, type ReactNode, type RefObject } from 'react';
import { Ban, ChevronLeft, CircleArrowRight, Clock, Repeat, Search, Sofa, Sun, Sunrise, X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Calendar, ChoiceCard, IconButton, Input, MenuList, Popover, Select, type MenuEntry } from '@ui';
import type { Recurrence } from '../api/gen/Recurrence';
import type { RepeatFrom } from '../api/gen/RepeatFrom';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { addDays, formatDay, localDay, quickPicks, repeatPresets, repeatText, sameRule, WEEKDAYS, weekdayOf, type QuickPick } from './model';
import { parseTodo } from './parse';
import { useMediaQuery } from '../common/useMediaQuery';

/** What the picker edits: a todo's due day, time and repeat rule. */
export type DueValue = { dueOn: string | null; dueTime: string | null; recurrence: Recurrence | null };
export type DuePanel = 'date' | 'time' | 'repeat';

/** A patch applied to a value, as the server would (clearing the day clears its time and rule). */
export function applyDuePatch(v: DueValue, p: TodoPatch): DueValue {
  const cleared = p.dueOn === '';
  return {
    dueOn: p.dueOn !== undefined ? p.dueOn || null : v.dueOn,
    dueTime: cleared ? null : p.dueTime !== undefined ? p.dueTime || null : v.dueTime,
    recurrence: cleared ? null : p.recurrence !== undefined ? p.recurrence : v.recurrence,
  };
}

/** A day for people: Today, Tomorrow, or "Wed 28 Oct". */
export function dayText(day: string, now: Date): string {
  const today = localDay(now);
  if (day === today) return 'Today';
  if (day === addDays(today, 1)) return 'Tomorrow';
  return formatDay(day, now);
}

const PICK_ICON: Record<QuickPick['id'], ReactNode> = {
  hour: <Clock aria-hidden className="text-running" />,
  today: <Sun aria-hidden className="text-waiting" />,
  tomorrow: <Sunrise aria-hidden className="text-waiting" />,
  weekend: <Sofa aria-hidden className="text-link" />,
  week: <CircleArrowRight aria-hidden className="text-link" />,
};

const SLOTS = [
  { time: '09:00', label: 'Morning' },
  { time: '12:00', label: 'Midday' },
  { time: '15:00', label: 'Afternoon' },
  { time: '18:00', label: 'Evening' },
];

const option =
  'flex h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-sm outline-none hover:bg-surface-2 focus-visible:bg-surface-2 focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch [&>svg]:size-3.5 [&>svg]:shrink-0';

export function DuePicker({ open, onOpenChange, anchor, value, now, onChange, onCustomRepeat, panel: startPanel = 'date', label }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** What opened it (the row's date, the calendar button, the ⋯ button, quick add's Date tag). */
  anchor: RefObject<Element | null>;
  value: DueValue;
  now: Date;
  /** A change, applied at once. */
  onChange: (p: TodoPatch) => void;
  /** Custom… in panel C: the full Repeat editor. */
  onCustomRepeat: () => void;
  /** The panel it opens on (quick add's Repeat tag opens C). */
  panel?: DuePanel;
  /** The todo's title, for the panel's name. */
  label?: string;
}) {
  return (
    <Popover anchor={anchor} open={open} onOpenChange={onOpenChange} label={label ? `Due date: ${label}` : 'Due date'} className="w-75 p-0">
      {/* Remounted each time it opens, so it starts on its first panel. */}
      {open && <Panels value={value} now={now} onChange={onChange} close={() => onOpenChange(false)} onCustomRepeat={onCustomRepeat} start={startPanel} />}
    </Popover>
  );
}

function Panels({ value, now, onChange, close, onCustomRepeat, start }: {
  value: DueValue;
  now: Date;
  onChange: (p: TodoPatch) => void;
  close: () => void;
  onCustomRepeat: () => void;
  start: DuePanel;
}) {
  const [panel, setPanel] = useState<DuePanel>(start);
  const today = localDay(now);
  const day = value.dueOn ?? today;
  if (panel === 'time') {
    return (
      <TimePanel
        day={day}
        time={value.dueTime}
        now={now}
        onBack={() => setPanel('date')}
        onTime={(dueTime) => onChange(dueTime && !value.dueOn ? { dueOn: today, dueTime } : { dueTime })}
      />
    );
  }
  if (panel === 'repeat') {
    return (
      <RepeatPanel
        day={day}
        current={value.recurrence}
        onBack={() => setPanel('date')}
        onRepeat={(recurrence) => onChange(recurrence && !value.dueOn ? { recurrence, dueOn: today } : { recurrence })}
        onCustom={() => {
          close();
          onCustomRepeat();
        }}
      />
    );
  }
  return (
    <DatePanel
      value={value}
      now={now}
      onPick={(p) => {
        onChange(p);
        close();
      }}
      onTime={() => setPanel('time')}
      onRepeat={() => setPanel('repeat')}
      onClear={onChange}
    />
  );
}

function DatePanel({ value, now, onPick, onTime, onRepeat, onClear }: {
  value: DueValue;
  now: Date;
  /** A day was chosen: apply it and close. */
  onPick: (p: TodoPatch) => void;
  onTime: () => void;
  onRepeat: () => void;
  /** A footer pill's ×: apply, stay open. */
  onClear: (p: TodoPatch) => void;
}) {
  const [typed, setTyped] = useState('');
  // On a touch screen the keyboard would open over the calendar (and shrink the popover), so it waits for a tap.
  const coarse = useMediaQuery('(pointer: coarse)');
  const today = localDay(now);
  const parsed = typed.trim() ? parseTodo(typed, now) : null;
  const understood = parsed && (parsed.dueOn || parsed.recurrence) ? parsed : null;
  const pickText = (q: QuickPick) =>
    q.dueTime ? `${dayText(q.dueOn, now)} ${q.dueTime}` : q.id === 'today' || q.id === 'tomorrow' ? WEEKDAYS[weekdayOf(q.dueOn)] : formatDay(q.dueOn, now);
  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2 border-b border-border px-3">
        <Search aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        <Input
          aria-label="Type a date"
          placeholder="Type a date: “fri 9am”, “in 2 weeks”"
          value={typed}
          autoFocus={!coarse}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== 'Enter' || !understood) return;
            e.preventDefault();
            onPick({
              dueOn: understood.dueOn ?? today,
              dueTime: understood.dueTime ?? '',
              ...(understood.recurrence ? { recurrence: understood.recurrence } : {}),
            });
          }}
          className="h-10 min-w-0 flex-1 border-0 bg-transparent px-0 shadow-none focus-visible:ring-0 dark:bg-transparent"
        />
        {understood?.dueOn && (
          <span className="shrink-0 text-xs text-muted-foreground">
            {dayText(understood.dueOn, now)}{understood.dueTime ? ` ${understood.dueTime}` : ''}
          </span>
        )}
      </div>
      <div className="flex flex-col p-1.5">
        {quickPicks(now).map((q) => (
          <button key={q.id} type="button" className={option} onClick={() => onPick({ dueOn: q.dueOn, dueTime: q.dueTime ?? '' })}>
            {PICK_ICON[q.id]}
            <span className="flex-1">{q.label}</span>
            <span className="text-xs text-muted-foreground">{pickText(q)}</span>
          </button>
        ))}
        {value.dueOn && (
          <button type="button" className={option} onClick={() => onPick({ dueOn: '' })}>
            <Ban aria-hidden className="text-muted-foreground" />
            <span className="flex-1">No date</span>
          </button>
        )}
      </div>
      <div className="border-t border-border px-2 pt-2.5 pb-2">
        <Calendar
          aria-label="Due date"
          value={value.dueOn}
          min={today}
          defaultFocus={today}
          onChange={(dueOn) => onPick({ dueOn })}
        />
      </div>
      <div className="flex gap-1.5 border-t border-border p-2">
        <Pill icon={<Clock aria-hidden />} text={value.dueTime ?? 'Time'} set={!!value.dueTime} label={value.dueTime ? `Time: ${value.dueTime}` : 'Set a time'} onOpen={onTime} onClear={() => onClear({ dueTime: '' })} clearLabel="No time" />
        <Pill
          icon={<Repeat aria-hidden />}
          text={value.recurrence ? repeatText(value.recurrence).replace(/^Every (?=[A-Z][a-z]{2}, )/, '') : 'Repeat'}
          set={!!value.recurrence}
          label={value.recurrence ? `Repeat: ${repeatText(value.recurrence)}` : 'Repeat'}
          onOpen={onRepeat}
          onClear={() => onClear({ recurrence: null })}
          clearLabel="Stop repeating"
        />
      </div>
    </div>
  );
}

/** A footer pill: opens its panel; × clears it when set. */
function Pill({ icon, text, set, label, onOpen, onClear, clearLabel }: {
  icon: ReactNode;
  text: string;
  set: boolean;
  label: string;
  onOpen: () => void;
  onClear: () => void;
  clearLabel: string;
}) {
  return (
    <div
      className={cn(
        'flex h-7.5 min-w-0 flex-1 items-center rounded-md border text-xs pointer-coarse:min-h-control-touch',
        set ? 'border-transparent bg-surface-2 text-foreground' : 'border-border text-muted-foreground',
      )}
    >
      <button type="button" aria-label={label} onClick={onOpen} className="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-md px-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring [&>svg]:size-3.5 [&>svg]:shrink-0">
        {icon}
        <span className="truncate">{text}</span>
      </button>
      {set && (
        <IconButton label={clearLabel} size="sm" noTooltip onClick={onClear} className="mr-0.5 size-6 shrink-0 text-muted-foreground">
          <X aria-hidden />
        </IconButton>
      )}
    </div>
  );
}

function PanelHeading({ children, onBack }: { children: ReactNode; onBack: () => void }) {
  return (
    <div className="flex h-9 items-center gap-1.5 border-b border-border px-1.5 text-xs font-semibold">
      <IconButton label="Back" size="sm" noTooltip onClick={onBack} className="text-muted-foreground">
        <ChevronLeft aria-hidden />
      </IconButton>
      <h3>{children}</h3>
    </div>
  );
}

function TimePanel({ day, time, now, onBack, onTime }: {
  day: string;
  time: string | null;
  now: Date;
  onBack: () => void;
  /** A time, or '' for none. */
  onTime: (time: string) => void;
}) {
  const [text, setText] = useState(time ?? '');
  const commit = () => {
    if (/^\d{2}:\d{2}$/.test(text) && text !== time) onTime(text);
  };
  return (
    <div className="flex flex-col">
      <PanelHeading onBack={onBack}>Time · {formatDay(day, now)}</PanelHeading>
      <div role="group" aria-label="Times" className="grid grid-cols-2 gap-1.5 p-2.5">
        {SLOTS.map((s) => (
          <ChoiceCard
            key={s.time}
            size="sm"
            selected={time === s.time}
            onClick={() => {
              setText(s.time);
              onTime(s.time);
            }}
            title={<span className="tabular-nums">{s.time}</span>}
            description={s.label}
            className="gap-0"
          />
        ))}
      </div>
      <div className="flex items-center gap-2 px-2.5 pb-2.5 text-xs text-muted-foreground">
        Or
        <Input
          type="time"
          aria-label="Time"
          size="sm"
          mono
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              commit();
            }
          }}
          className="w-28"
        />
        {time && (
          <button
            type="button"
            onClick={() => {
              setText('');
              onTime('');
            }}
            className="ml-auto rounded-sm px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch"
          >
            No time
          </button>
        )}
      </div>
      <p className="border-t border-border px-2.5 py-2.5 text-xs text-muted-foreground">
        The reminder comes at this time instead of the start of the day.
      </p>
    </div>
  );
}

function RepeatPanel({ day, current, onBack, onRepeat, onCustom }: {
  day: string;
  current: Recurrence | null;
  onBack: () => void;
  onRepeat: (r: Recurrence | null) => void;
  onCustom: () => void;
}) {
  const [from, setFrom] = useState<RepeatFrom>(current?.from ?? 'due');
  const presets = repeatPresets(day);
  const isPreset = !!current && presets.some((p) => sameRule(p.rule, current.rule));
  const entries: MenuEntry[] = [
    { label: 'Doesn’t repeat', checked: !current, onSelect: () => onRepeat(null) },
    ...presets.map((p): MenuEntry => {
      const on = !!current && sameRule(p.rule, current.rule);
      return { label: p.label, detail: p.detail, checked: on, onSelect: () => onRepeat({ rule: p.rule, from }) };
    }),
    ...(current && !isPreset ? [{ label: repeatText(current), checked: true, onSelect: () => onRepeat(current) }] : []),
    { separator: true },
    { label: 'Custom…', onSelect: onCustom },
  ];
  return (
    <div className="flex flex-col">
      <PanelHeading onBack={onBack}>Repeat</PanelHeading>
      <MenuList label="Repeat" entries={entries} className="p-1.5" />
      <div className="flex items-center gap-2 border-t border-border px-2.5 py-2 text-xs text-muted-foreground">
        Next one counts from
        <Select<RepeatFrom>
          aria-label="Next one counts from"
          size="sm"
          quiet
          value={from}
          onChange={(f) => {
            setFrom(f);
            if (current && current.from !== f) onRepeat({ ...current, from: f });
          }}
          options={[{ value: 'due', label: 'the due date' }, { value: 'done', label: 'when it’s done' }]}
        />
      </div>
    </div>
  );
}
