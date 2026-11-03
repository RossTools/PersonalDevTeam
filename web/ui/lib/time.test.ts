import { describe, expect, it } from 'vitest';
import { absoluteTime, clockTime, formatDuration, relativeTime, shortAge, shortDate } from './time';

const NOW = new Date(2026, 9, 17, 12, 0).getTime();
const H = 3_600_000;

describe('relativeTime', () => {
  it('steps from just now to minutes, hours, days and dates', () => {
    expect(relativeTime(NOW - 10_000, NOW)).toBe('just now');
    expect(relativeTime(NOW - 5 * 60_000, NOW)).toBe('5m ago');
    expect(relativeTime(NOW - 2 * H, NOW)).toBe('2h ago');
    expect(relativeTime(NOW - 3 * 24 * H, NOW)).toBe('3d ago');
    expect(relativeTime(NOW - 30 * 24 * H, NOW)).toBe(shortDate(NOW - 30 * 24 * H, NOW));
    expect(relativeTime(new Date(2024, 0, 5).getTime(), NOW)).toContain('2024');
  });

  it('treats future times as just now', () => {
    expect(relativeTime(NOW + H, NOW)).toBe('just now');
  });
});

describe('shortAge', () => {
  it('is now, minutes, hours, then days', () => {
    expect(shortAge(NOW - 10_000, NOW)).toBe('now');
    expect(shortAge(NOW - 10 * 60_000, NOW)).toBe('10m');
    expect(shortAge(NOW - 3 * H, NOW)).toBe('3h');
    expect(shortAge(NOW - 50 * H, NOW)).toBe('2d');
  });
});

describe('shortDate', () => {
  it('names the year only in another year', () => {
    expect(shortDate(new Date(2026, 2, 12).getTime(), NOW)).not.toContain('2026');
    expect(shortDate(new Date(2025, 2, 12).getTime(), NOW)).toContain('2025');
  });
});

describe('absoluteTime', () => {
  it('gives the date and the time', () => {
    expect(absoluteTime(NOW)).toContain('2026');
  });
});

describe('clockTime', () => {
  it('gives the hour and minute, with the weekday on another day when asked', () => {
    const same = clockTime(NOW + H, { weekday: true, now: NOW });
    expect(same).toBe(clockTime(NOW + H));
    const later = clockTime(NOW + 48 * H, { weekday: true, now: NOW });
    expect(later).not.toBe(clockTime(NOW + 48 * H));
    expect(later).toContain(new Date(NOW + 48 * H).toLocaleDateString(undefined, { weekday: 'short' }));
  });
});

describe('formatDuration', () => {
  it('reads in ms, s, min and h', () => {
    expect(formatDuration(850)).toBe('850 ms');
    expect(formatDuration(12_000)).toBe('12 s');
    expect(formatDuration(184_000)).toBe('3 min 4 s');
    expect(formatDuration(180_000)).toBe('3 min');
    expect(formatDuration(2 * H + 5 * 60_000)).toBe('2 h 5 min');
    expect(formatDuration(2 * H)).toBe('2 h');
  });
});
