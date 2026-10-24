import { describe, expect, it } from 'vitest';
import { parseTodo, type Parsed } from './parse';

// Wednesday 21 Oct 2026, 10:00 local.
const WED = new Date(2026, 9, 21, 10, 0);
const FRI = new Date(2026, 9, 23, 10, 0);
const SAT = new Date(2026, 9, 24, 10, 0);
const SUN = new Date(2026, 9, 25, 10, 0);

function parse(text: string, now = WED, ignore?: ReadonlySet<'when' | 'repeat'>): Parsed {
  const p = parseTodo(text, now, ignore);
  // Every match's range is the text it reports, and matches are in text order.
  for (const m of p.matches) expect(text.slice(m.start, m.end)).toBe(m.text);
  const starts = p.matches.map((m) => m.start);
  expect(starts).toEqual([...starts].sort((a, b) => a - b));
  return p;
}

const due = (text: string, now = WED) => {
  const p = parse(text, now);
  return [p.dueOn, p.dueTime];
};

describe('days', () => {
  it('reads today, tonight and tomorrow', () => {
    expect(due('Ship it today')).toEqual(['2026-10-21', undefined]);
    expect(due('Ship it tonight')).toEqual(['2026-10-21', '20:00']);
    expect(due('Ship it tonight 9pm')).toEqual(['2026-10-21', '21:00']);
    expect(due('Ship it tomorrow')).toEqual(['2026-10-22', undefined]);
    expect(due('Ship it TMR')).toEqual(['2026-10-22', undefined]);
  });

  it('reads weekdays as the coming one, never today', () => {
    expect(due('Pay rent fri')).toEqual(['2026-10-23', undefined]);
    expect(due('Pay rent Friday')).toEqual(['2026-10-23', undefined]);
    expect(due('Pay rent on fri')).toEqual(['2026-10-23', undefined]);
    expect(due('Pay rent next fri')).toEqual(['2026-10-23', undefined]);
    expect(due('Pay rent wed')).toEqual(['2026-10-28', undefined]);
    expect(due('Pay rent mon')).toEqual(['2026-10-26', undefined]);
    expect(due('Pay rent fri', FRI)).toEqual(['2026-10-30', undefined]);
    expect(due('Pay rent next fri', FRI)).toEqual(['2026-10-30', undefined]);
  });

  it('reads sat and sun only after on/next/this', () => {
    expect(due('Mow on sat')).toEqual(['2026-10-24', undefined]);
    expect(due('Mow next sun')).toEqual(['2026-10-25', undefined]);
    expect(due('Mow this sat')).toEqual(['2026-10-24', undefined]);
    expect(due('Mow saturday')).toEqual(['2026-10-24', undefined]);
    expect(parse('Mow sat').matches).toEqual([]);
  });

  it('reads next week as the coming Monday', () => {
    expect(due('Plan next week')).toEqual(['2026-10-26', undefined]);
    expect(due('Plan next week', new Date(2026, 9, 26, 10))).toEqual(['2026-11-02', undefined]);
  });

  it('reads this weekend as the coming Saturday', () => {
    expect(due('Hike this weekend')).toEqual(['2026-10-24', undefined]);
    expect(due('Hike this weekend', SAT)).toEqual(['2026-10-31', undefined]);
    expect(due('Hike this weekend', SUN)).toEqual(['2026-10-31', undefined]);
  });

  it('reads in N days, weeks and months', () => {
    expect(due('Renew in 3 days')).toEqual(['2026-10-24', undefined]);
    expect(due('Renew in 1 day')).toEqual(['2026-10-22', undefined]);
    expect(due('Renew in a week')).toEqual(['2026-10-28', undefined]);
    expect(due('Renew in 2 weeks')).toEqual(['2026-11-04', undefined]);
    expect(due('Renew in 1 month')).toEqual(['2026-11-21', undefined]);
    expect(due('Renew in 2 months')).toEqual(['2026-12-21', undefined]);
    expect(due('Renew in a month', new Date(2027, 0, 31, 10))).toEqual(['2027-02-28', undefined]);
  });

  it('reads dates, this year while still ahead, else next year', () => {
    for (const t of ['nov 3', '3 nov', 'November 3rd', '3rd of November', 'on Nov 3', 'the 3rd of nov']) {
      const p = parse(`Vote ${t}`);
      expect(p.dueOn, t).toBe('2026-11-03');
      expect(p.title, t).toBe('Vote');
    }
    expect(due('Vote oct 21')).toEqual(['2026-10-21', undefined]);
    expect(due('Vote oct 1')).toEqual(['2027-10-01', undefined]);
    expect(due('Plant may 3')).toEqual(['2027-05-03', undefined]);
    expect(due('Plant 3 may')).toEqual(['2027-05-03', undefined]);
    expect(due('Leap feb 29')).toEqual(['2028-02-29', undefined]);
  });

  it('rejects days no month has', () => {
    const p = parse('Pay feb 30');
    expect(p.matches).toEqual([]);
    expect(p.dueOn).toBeUndefined();
    expect(p.title).toBe('Pay feb 30');
  });
});

describe('times', () => {
  it('reads 12-hour, 24-hour, noon and midnight', () => {
    expect(due('Call 3pm')).toEqual(['2026-10-21', '15:00']);
    expect(due('Call 3:30pm')).toEqual(['2026-10-21', '15:30']);
    expect(due('Call 3:30 PM')).toEqual(['2026-10-21', '15:30']);
    expect(due('Call at 15:30')).toEqual(['2026-10-21', '15:30']);
    expect(due('Call 15:30')).toEqual(['2026-10-21', '15:30']);
    expect(due('Call noon')).toEqual(['2026-10-21', '12:00']);
    expect(due('Call 12pm')).toEqual(['2026-10-21', '12:00']);
  });

  it('rolls a time already past over to tomorrow', () => {
    expect(due('Call 9am')).toEqual(['2026-10-22', '09:00']);
    expect(due('Call 9 am')).toEqual(['2026-10-22', '09:00']);
    expect(due('Call at 10:00')).toEqual(['2026-10-22', '10:00']);
    expect(due('Call midnight')).toEqual(['2026-10-22', '00:00']);
    expect(due('Call 12am')).toEqual(['2026-10-22', '00:00']);
  });

  it('pairs a day and a time in either order', () => {
    for (const t of ['Call mom tomorrow 3pm', 'Call mom 3pm tomorrow', 'Call mom tomorrow at 3pm', 'Call mom at 3pm tomorrow']) {
      const p = parse(t);
      expect([p.dueOn, p.dueTime, p.title], t).toEqual(['2026-10-22', '15:00', 'Call mom']);
      expect(p.matches.every((m) => m.kind === 'when'), t).toBe(true);
    }
    const p = parse('Standup fri at 9am');
    expect([p.dueOn, p.dueTime, p.title]).toEqual(['2026-10-23', '09:00', 'Standup']);
    expect(p.matches.map((m) => m.text)).toEqual(['fri', 'at 9am']);
  });
});

describe('repeats', () => {
  const rec = (text: string, now = WED) => {
    const p = parse(text, now);
    return [p.recurrence?.rule, p.dueOn];
  };

  it('repeats daily', () => {
    expect(rec('Stretch every day')).toEqual([{ every: 'day', interval: 1 }, '2026-10-21']);
    expect(rec('Stretch daily')).toEqual([{ every: 'day', interval: 1 }, '2026-10-21']);
    expect(rec('Water plants every 3 days')).toEqual([{ every: 'day', interval: 3 }, '2026-10-21']);
    expect(parse('Stretch daily').recurrence?.from).toBe('due');
  });

  it('repeats on weekdays, starting with the next one', () => {
    expect(rec('Standup every weekday')).toEqual([{ every: 'weekday' }, '2026-10-21']);
    expect(rec('Standup every weekday', SAT)).toEqual([{ every: 'weekday' }, '2026-10-26']);
  });

  it('repeats weekly on the due day, or today', () => {
    expect(rec('Review every week')).toEqual([{ every: 'week', interval: 1, days: [3] }, '2026-10-21']);
    expect(rec('Review weekly')).toEqual([{ every: 'week', interval: 1, days: [3] }, '2026-10-21']);
    expect(rec('Review every 2 weeks')).toEqual([{ every: 'week', interval: 2, days: [3] }, '2026-10-21']);
    expect(rec('Review every 2 weeks fri')).toEqual([{ every: 'week', interval: 2, days: [5] }, '2026-10-23']);
    expect(rec('Review fri weekly')).toEqual([{ every: 'week', interval: 1, days: [5] }, '2026-10-23']);
  });

  it('repeats on named weekdays, starting with the first of them', () => {
    expect(rec('Gym every mon, thu')).toEqual([{ every: 'week', interval: 1, days: [1, 4] }, '2026-10-22']);
    expect(rec('Gym every thu and mon')).toEqual([{ every: 'week', interval: 1, days: [1, 4] }, '2026-10-22']);
    expect(rec('Gym every monday')).toEqual([{ every: 'week', interval: 1, days: [1] }, '2026-10-26']);
    expect(rec('Gym every wed')).toEqual([{ every: 'week', interval: 1, days: [3] }, '2026-10-21']);
    expect(rec('Gym every sat')).toEqual([{ every: 'week', interval: 1, days: [6] }, '2026-10-24']);
    expect(rec('Gym every sat and sun')).toEqual([{ every: 'week', interval: 1, days: [6, 7] }, '2026-10-24']);
    const p = parse('Gym every mon, thu');
    expect(p.matches).toEqual([{ kind: 'repeat', start: 4, end: 18, text: 'every mon, thu' }]);
    expect(p.title).toBe('Gym');
  });

  it('repeats monthly on the due day of the month, or today', () => {
    expect(rec('Pay rent every month')).toEqual([{ every: 'month', day: 21 }, '2026-10-21']);
    expect(rec('Pay rent monthly')).toEqual([{ every: 'month', day: 21 }, '2026-10-21']);
    expect(rec('Pay rent monthly nov 3')).toEqual([{ every: 'month', day: 3 }, '2026-11-03']);
  });

  it('starts a repeat with a past time tomorrow', () => {
    const p = parse('Stretch every day 9am');
    expect([p.dueOn, p.dueTime]).toEqual(['2026-10-22', '09:00']);
  });
});

describe('the title and matches', () => {
  it('takes every phrase out of the title', () => {
    const p = parse('Call the dentist tomorrow 3pm every month');
    expect(p.title).toBe('Call the dentist');
    expect(p.dueOn).toBe('2026-10-22');
    expect(p.dueTime).toBe('15:00');
    expect(p.recurrence).toEqual({ rule: { every: 'month', day: 22 }, from: 'due' });
    expect(p.matches.map((m) => [m.kind, m.text])).toEqual([['when', 'tomorrow'], ['when', '3pm'], ['repeat', 'every month']]);
  });

  it('tidies separators left behind', () => {
    expect(parse('Pay rent, tomorrow').title).toBe('Pay rent');
    expect(parse('Call mom at 3pm').title).toBe('Call mom');
    expect(parse('Tomorrow: call mom').title).toBe('call mom');
    expect(parse('Call mom tomorrow, about the cake').title).toBe('Call mom, about the cake');
    expect(parse('Buy   milk   today').title).toBe('Buy milk');
  });

  it('keeps the text as the title when nothing else is left', () => {
    const p = parse('tomorrow');
    expect(p.title).toBe('tomorrow');
    expect(p.dueOn).toBe('2026-10-22');
  });

  it('prefers the phrase nearest the end', () => {
    const p = parse('Move the tuesday sync to friday');
    expect(p.dueOn).toBe('2026-10-23');
    expect(p.matches.map((m) => m.text)).toEqual(['friday']);
    expect(p.title).toBe('Move the tuesday sync to');
  });

  it('matches nothing in ordinary text', () => {
    for (const t of ['Read 3 chapters', 'Buy 2 apples', 'May the force be with you', 'Satisfy the sunny customer',
      'Sat on the chair', 'Tuesdays are long', 'Meet at 3', 'Monday-ish plan', 'Fix the 404', 'Weekday planning', 'Thursdays']) {
      const p = parse(t);
      expect(p.matches, t).toEqual([]);
      expect([p.title, p.dueOn, p.dueTime, p.recurrence], t).toEqual([t, undefined, undefined, undefined]);
    }
  });

  it('leaves dismissed kinds in the title', () => {
    const noWhen = parse('Buy milk tomorrow 3pm every day', WED, new Set(['when']));
    expect(noWhen.title).toBe('Buy milk tomorrow 3pm');
    expect(noWhen.matches.map((m) => m.kind)).toEqual(['repeat']);
    expect(noWhen.dueTime).toBeUndefined();
    expect(noWhen.dueOn).toBe('2026-10-21');

    const noRepeat = parse('Gym every mon', WED, new Set(['repeat']));
    expect(noRepeat.title).toBe('Gym every mon');
    expect(noRepeat.matches).toEqual([]);
    expect(noRepeat.recurrence).toBeUndefined();
    expect(noRepeat.dueOn).toBeUndefined();

    const both = parse('Call tomorrow daily', WED, new Set(['when', 'repeat']));
    expect([both.title, both.matches, both.dueOn]).toEqual(['Call tomorrow daily', [], undefined]);
  });
});
