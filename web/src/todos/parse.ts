// The quick-add box's natural-language dates (PDT-143): finds a due day ("tomorrow", "fri", "nov 3", "in 2 weeks"), a
// time ("3pm", "15:30", "noon") and a repeat ("every mon, thu", "monthly") in a todo's text, and returns them with the
// title left once those phrases are taken out. Pure: `now` is passed in.
import type { Recurrence } from '../api/gen/Recurrence';
import type { RepeatRule } from '../api/gen/RepeatRule';
import { addDays, localDay, weekdayOf } from './model';

/** What a phrase set: the due day/time ('when') or the repeat rule ('repeat'). */
export type ParseKind = 'when' | 'repeat';
interface ParseMatch { kind: ParseKind; start: number; end: number; text: string } // [start,end) char range in the input
export interface Parsed {
  /** The text with matched phrases removed and whitespace tidied (collapse runs, trim, drop dangling separators like a trailing "," or " at"/" on" left behind). Falls back to the original text, trimmed, when stripping leaves it empty. */
  title: string;
  dueOn?: string;     // 'YYYY-MM-DD' local
  dueTime?: string;   // 'HH:MM' 24h
  recurrence?: Recurrence;
  matches: ParseMatch[]; // in text order, for highlighting
}

// --- Words ---------------------------------------------------------------------------------------------------------

// Weekday names by number (1 = Monday … 7 = Sunday). Bare "sat" and "sun" are ordinary words too, so they only count
// after on/next/this/every.
const WEEKDAY_NAMES: readonly [number, string[]][] = [
  [1, ['monday', 'mon']],
  [2, ['tuesday', 'tues', 'tue']],
  [3, ['wednesday', 'wed']],
  [4, ['thursday', 'thurs', 'thur', 'thu']],
  [5, ['friday', 'fri']],
  [6, ['saturday', 'sat']],
  [7, ['sunday', 'sun']],
];
const AMBIGUOUS_WEEKDAYS = new Set(['sat', 'sun']);
const WD_ALL = WEEKDAY_NAMES.flatMap(([, names]) => names).join('|');
const WD_SAFE = WEEKDAY_NAMES.flatMap(([, names]) => names.filter((n) => !AMBIGUOUS_WEEKDAYS.has(n))).join('|');

function weekdayNumber(word: string): number {
  const w = word.toLowerCase();
  return WEEKDAY_NAMES.find(([, names]) => names.includes(w))![0];
}

// Months (1 = January). They only count next to a day number, so "may" and "mar" are safe.
const MONTH_NAMES: readonly string[][] = [
  ['january', 'jan'], ['february', 'feb'], ['march', 'mar'], ['april', 'apr'], ['may'], ['june', 'jun'],
  ['july', 'jul'], ['august', 'aug'], ['september', 'sept', 'sep'], ['october', 'oct'], ['november', 'nov'],
  ['december', 'dec'],
];
const MONTHS = MONTH_NAMES.flat().join('|');

function monthNumber(word: string): number {
  const w = word.toLowerCase();
  return MONTH_NAMES.findIndex((names) => names.includes(w)) + 1;
}

const DAY_NUM = '(3[01]|[12]\\d|0?[1-9])(?:st|nd|rd|th)?';

/** A whole-word pattern: not inside a word (letters, digits, apostrophes, hyphens or a time's colon). */
function rx(source: string): RegExp {
  return new RegExp(`(?<![\\p{L}\\p{N}_'’:-])(?:${source})(?![\\p{L}\\p{N}_'’-])(?!:\\d)`, 'giu');
}

// --- Phrases -------------------------------------------------------------------------------------------------------

type RepeatSpec =
  | { every: 'day'; interval: number }
  | { every: 'weekday' }
  | { every: 'week'; interval: number; days?: number[] }
  | { every: 'month' };

/** What a phrase means: a due day (maybe with a time, for "tonight"), a time or a repeat. */
type Value =
  | { group: 'day'; day: string; time?: string }
  | { group: 'time'; time: string }
  | { group: 'repeat'; spec: RepeatSpec };
type Candidate = Value & { start: number; end: number };

const pad = (n: number) => String(n).padStart(2, '0');

/** The coming `weekday` after `today`, never today itself. */
function comingWeekday(today: string, weekday: number): string {
  return addDays(today, ((weekday - weekdayOf(today) + 7) % 7) || 7);
}

function addMonths(now: Date, n: number): string {
  const y = now.getFullYear();
  const m = now.getMonth() + n;
  const last = new Date(y, m + 1, 0).getDate();
  return localDay(new Date(y, m, Math.min(now.getDate(), last)));
}

/** The next `month`/`day` on or after today (this year, else the next year it exists); null for a day no month has. */
function nextDate(today: string, month: number, day: number): string | null {
  const year = Number(today.slice(0, 4));
  for (let y = year; y <= year + 8; y++) {
    const d = new Date(y, month - 1, day);
    if (d.getMonth() !== month - 1) continue; // feb 30, or feb 29 off a leap year
    const s = localDay(d);
    if (s >= today) return s;
  }
  return null;
}

function count(word: string): number {
  return /^(a|an|one)$/i.test(word) ? 1 : Number(word);
}

/** Each phrase family: a pattern and what a match of it means (null: not a real date after all). */
function families(now: Date): [RegExp, (m: RegExpMatchArray) => Value | null][] {
  const today = localDay(now);
  const day = (d: string | null, time?: string): Value | null => (d ? { group: 'day', day: d, ...(time ? { time } : {}) } : null);
  const time = (h: number, min: number): Value | null => (h < 24 && min < 60 ? { group: 'time', time: `${pad(h)}:${pad(min)}` } : null);
  const repeat = (spec: RepeatSpec): Value => ({ group: 'repeat', spec });
  return [
    // Days.
    [rx('today'), () => day(today)],
    [rx('tonight'), () => day(today, '20:00')],
    [rx('tomorrow|tmrw?'), () => day(addDays(today, 1))],
    [rx(`(?:on|next|this)\\s+(${WD_ALL})`), (m) => day(comingWeekday(today, weekdayNumber(m[1]!)))],
    [rx(`(${WD_SAFE})`), (m) => day(comingWeekday(today, weekdayNumber(m[1]!)))],
    [rx('next\\s+week'), () => day(comingWeekday(today, 1))],
    [rx('this\\s+weekend'), () => day(comingWeekday(today, 6))],
    [rx('in\\s+(\\d{1,3}|an?|one)\\s+(day|week|month)s?'), (m) => {
      const n = count(m[1]!);
      const unit = m[2]!.toLowerCase();
      return day(unit === 'month' ? addMonths(now, n) : addDays(today, unit === 'week' ? n * 7 : n));
    }],
    [rx(`(?:on\\s+)?(${MONTHS})\\.?\\s+${DAY_NUM}`), (m) => day(nextDate(today, monthNumber(m[1]!), Number(m[2])))],
    [rx(`(?:on\\s+)?(?:the\\s+)?${DAY_NUM}\\s+(?:of\\s+)?(${MONTHS})`), (m) => day(nextDate(today, monthNumber(m[2]!), Number(m[1])))],
    // Times.
    [rx('(?:at\\s+)?(1[0-2]|0?[1-9])(?::([0-5]\\d))?\\s?(am|pm)'), (m) => {
      const h = Number(m[1]) % 12 + (m[3]!.toLowerCase() === 'pm' ? 12 : 0);
      return time(h, Number(m[2] ?? 0));
    }],
    [rx('(?:at\\s+)?([01]?\\d|2[0-3]):([0-5]\\d)'), (m) => time(Number(m[1]), Number(m[2]))],
    [rx('(?:at\\s+)?noon'), () => time(12, 0)],
    [rx('(?:at\\s+)?midnight'), () => time(0, 0)],
    // Repeats.
    [rx('every\\s+day|daily'), () => repeat({ every: 'day', interval: 1 })],
    [rx('every\\s+(\\d{1,3})\\s+days'), (m) => (Number(m[1]) > 0 ? repeat({ every: 'day', interval: Number(m[1]) }) : null)],
    [rx('every\\s+weekday'), () => repeat({ every: 'weekday' })],
    [rx('every\\s+week|weekly'), () => repeat({ every: 'week', interval: 1 })],
    [rx('every\\s+(\\d{1,3}|other)\\s+weeks?'), (m) => {
      const n = m[1]!.toLowerCase() === 'other' ? 2 : Number(m[1]);
      return n > 0 ? repeat({ every: 'week', interval: n }) : null;
    }],
    [rx('every\\s+month|monthly'), () => repeat({ every: 'month' })],
    [rx(`every\\s+(?:${WD_ALL})(?:\\s*(?:,\\s*and|,|and|&)\\s*(?:${WD_ALL}))*`), (m) => {
      const names = m[0].slice(5).match(new RegExp(`(?<![\\p{L}])(?:${WD_ALL})(?![\\p{L}])`, 'giu')) ?? [];
      const days = [...new Set(names.map(weekdayNumber))].sort((a, b) => a - b);
      return repeat({ every: 'week', interval: 1, days });
    }],
  ];
}

function candidates(text: string, now: Date): Candidate[] {
  const out: Candidate[] = [];
  for (const [re, meaning] of families(now)) {
    for (const m of text.matchAll(re)) {
      const value = meaning(m);
      if (value) out.push({ ...value, start: m.index!, end: m.index! + m[0].length });
    }
  }
  return out;
}

const overlaps = (a: { start: number; end: number }, b: { start: number; end: number }) => a.start < b.end && b.start < a.end;

/** The group's candidate nearest the end (the longest when two end together) that doesn't overlap a taken one. */
function pick<G extends Candidate['group']>(all: Candidate[], group: G, taken: Candidate[]): Extract<Candidate, { group: G }> | undefined {
  return all
    .filter((c): c is Extract<Candidate, { group: G }> => c.group === group && !taken.some((t) => overlaps(t, c)))
    .sort((a, b) => b.end - a.end || a.start - b.start)[0];
}

// --- Repeats -------------------------------------------------------------------------------------------------------

/** The first day on or after `from` that the repeat falls on. */
function firstDay(spec: RepeatSpec, from: string): string {
  const fits = (d: string) =>
    spec.every === 'weekday' ? weekdayOf(d) <= 5 : spec.every === 'week' && spec.days ? spec.days.includes(weekdayOf(d)) : true;
  let d = from;
  for (let i = 0; i < 7 && !fits(d); i++) d = addDays(d, 1);
  return d;
}

/** The rule, anchored on the due day (its weekday, its day of the month). */
function ruleFor(spec: RepeatSpec, due: string): RepeatRule {
  switch (spec.every) {
    case 'day':
      return { every: 'day', interval: spec.interval };
    case 'weekday':
      return { every: 'weekday' };
    case 'week':
      return { every: 'week', interval: spec.interval, days: spec.days ?? [weekdayOf(due)] };
    case 'month':
      return { every: 'month', day: Number(due.slice(8, 10)) };
  }
}

// --- Title ---------------------------------------------------------------------------------------------------------

const SEPARATORS = '[\\s,;:\\-–—]';

function strip(text: string, ranges: { start: number; end: number }[]): string {
  let out = '';
  let at = 0;
  for (const r of ranges) {
    out += `${text.slice(at, r.start)} `;
    at = r.end;
  }
  out += text.slice(at);
  let prev: string;
  do {
    prev = out;
    out = out
      .replace(/\s+/g, ' ')
      .replace(/\s+([,;:.!?])/g, '$1')
      .replace(/([,;:])(?=[,;:.!?])/g, '')
      .replace(new RegExp(`^${SEPARATORS}+|${SEPARATORS}+$`, 'g'), '')
      .replace(/\s+(?:at|on|by|due)$/i, '');
  } while (out !== prev);
  return out;
}

// --- Parse ---------------------------------------------------------------------------------------------------------

/** `ignore`: kinds the user dismissed (× on a tag): those phrases are not matched and stay in the title. */
export function parseTodo(text: string, now: Date, ignore?: ReadonlySet<ParseKind>): Parsed {
  const all = candidates(text, now);
  // A repeat goes first so "every mon" isn't read as a day; a dismissed one still holds its words.
  const repeat = pick(all, 'repeat', []);
  const day = pick(all, 'day', repeat ? [repeat] : []);
  const time = pick(all, 'time', [...(repeat ? [repeat] : []), ...(day ? [day] : [])]);

  const useWhen = !ignore?.has('when');
  const useRepeat = !!repeat && !ignore?.has('repeat');
  const chosen: { kind: ParseKind; c: Candidate }[] = [];
  if (useWhen && day) chosen.push({ kind: 'when', c: day });
  if (useWhen && time) chosen.push({ kind: 'when', c: time });
  if (useRepeat) chosen.push({ kind: 'repeat', c: repeat });
  chosen.sort((a, b) => a.c.start - b.c.start);

  const result: Parsed = {
    title: strip(text, chosen.map((x) => x.c)) || text.trim(),
    matches: chosen.map(({ kind, c }) => ({ kind, start: c.start, end: c.end, text: text.slice(c.start, c.end) })),
  };

  const today = localDay(now);
  const hm = `${pad(now.getHours())}:${pad(now.getMinutes())}`;
  if (useWhen) {
    result.dueTime = time?.time ?? day?.time;
    // A time alone: today while it's still ahead, else tomorrow.
    result.dueOn = day?.day ?? (time ? (time.time > hm ? today : addDays(today, 1)) : undefined);
  }
  if (useRepeat) {
    const due = (useWhen && day?.day) || firstDay(repeat.spec, result.dueOn ?? today);
    result.dueOn = due;
    result.recurrence = { rule: ruleFor(repeat.spec, due), from: 'due' };
  }
  if (result.dueOn === undefined) delete result.dueOn;
  if (result.dueTime === undefined) delete result.dueTime;
  return result;
}
