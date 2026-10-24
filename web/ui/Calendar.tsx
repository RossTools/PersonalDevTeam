import { CalendarDate, parseDate } from '@internationalized/date';
import { Calendar as CalendarPrimitive } from './primitives/calendar';

export interface CalendarProps {
  /** The chosen day, `YYYY-MM-DD` (or null). */
  value: string | null;
  /** A day was picked (`YYYY-MM-DD`): a click, or Enter on the focused day. */
  onChange: (day: string) => void;
  /** Days before this one (`YYYY-MM-DD`) are muted and can't be picked, except the chosen day. */
  min?: string;
  /** The month shown first when nothing is chosen (`YYYY-MM-DD`; default today). */
  defaultFocus?: string;
  /** Focus the chosen (or first) day when it mounts. */
  autoFocus?: boolean;
  'aria-label'?: string;
  className?: string;
}

const day = (s: string | null | undefined): CalendarDate | null => {
  if (!s || !/^\d{4}-\d{2}-\d{2}$/.test(s)) return null;
  try {
    return parseDate(s);
  } catch {
    return null;
  }
};

/**
 * A month of days to pick one from (the due-date picker): Monday first, ‹ › to page, arrows move, Enter picks. Days
 * are plain `YYYY-MM-DD` strings, the API's own form.
 */
export function Calendar({ value, onChange, min, defaultFocus, autoFocus, className, ...aria }: CalendarProps) {
  const chosen = day(value);
  const floor = day(min);
  return (
    <CalendarPrimitive
      aria-label={aria['aria-label'] ?? 'Date'}
      value={chosen}
      onChange={(d) => onChange(d.toString())}
      defaultFocusedValue={chosen ?? day(defaultFocus) ?? undefined}
      isDateUnavailable={floor ? (d) => d.compare(floor) < 0 && !(chosen && d.compare(chosen) === 0) : undefined}
      autoFocus={autoFocus}
      className={className}
    />
  );
}
