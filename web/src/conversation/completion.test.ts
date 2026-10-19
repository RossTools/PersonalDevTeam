import { describe, expect, it } from 'vitest';
import { applyCompletion, commandSuggestions, completionAt, limitWarning, windowLabel } from './completion';

describe('completionAt', () => {
  it('finds a /command only as the first word, and @files anywhere a word starts', () => {
    expect(completionAt('/comp', 5)).toEqual({ kind: 'command', start: 0, query: 'comp' });
    expect(completionAt('/', 1)).toEqual({ kind: 'command', start: 0, query: '' });
    expect(completionAt('/compact now', 12)).toBeNull();
    expect(completionAt('see /x', 6)).toBeNull();
    expect(completionAt('look at @src/ma', 15)).toEqual({ kind: 'file', start: 8, query: 'src/ma' });
    expect(completionAt('@', 1)).toEqual({ kind: 'file', start: 0, query: '' });
    expect(completionAt('mail me@x', 9)).toBeNull();
    expect(completionAt('@a b', 4)).toBeNull();
  });

  it('replaces the word and keeps what follows', () => {
    const c = completionAt('fix @mai please', 8)!;
    expect(applyCompletion('fix @mai please', 8, c, '@src/main.rs')).toEqual({ text: 'fix @src/main.rs please', caret: 17 });
    const d = completionAt('/co', 3)!;
    expect(applyCompletion('/co', 3, d, '/compact')).toEqual({ text: '/compact ', caret: 9 });
    // The caret was mid-word: the rest of the word goes too.
    expect(applyCompletion('@mainrs x', 3, completionAt('@mainrs x', 3)!, '@main.rs').text).toBe('@main.rs x');
  });
});

describe('commandSuggestions', () => {
  const cmds = [
    { name: 'compact', description: 'Compact the conversation' },
    { name: 'review', description: 'Review changes', argumentHint: '[pr]' },
    { name: 'config', description: 'Settings' },
  ];
  it('lists prefix matches first and marks TUI-only commands', () => {
    const s = commandSuggestions(cmds, ['config', 'login'], 'co');
    expect(s.map((x) => x.label)).toEqual(['/compact', '/config']);
    expect(s[1]).toMatchObject({ disabled: true, hint: 'Only in the terminal' });
    expect(commandSuggestions(cmds, ['login'], '').map((x) => x.label)).toEqual(['/compact', '/review', '/config', '/login']);
    expect(commandSuggestions(cmds, [], 'view')[0]).toMatchObject({ label: '/review', detail: '[pr] Review changes' });
  });
});

describe('rate limits', () => {
  const now = new Date('2026-10-19T10:00:00').getTime();
  it('names windows the way people do', () => {
    expect(windowLabel('five_hour')).toBe('5-hour');
    expect(windowLabel('seven_day')).toBe('weekly');
    expect(windowLabel('300_minutes')).toBe('5-hour');
    expect(windowLabel('10080_minutes')).toBe('weekly');
    expect(windowLabel('1440_minutes')).toBe('1-day');
  });

  it('warns from 80 % and says when a hit limit resets', () => {
    const later = new Date('2026-10-19T15:40:00').getTime();
    expect(limitWarning([{ name: 'five_hour', utilization: 0.5, resetsAt: later }], now)).toBeNull();
    const w = limitWarning([{ name: 'five_hour', utilization: 0.5, resetsAt: later }, { name: 'seven_day', utilization: 0.86, resetsAt: later }], now)!;
    expect(w.reached).toBe(false);
    expect(w.text).toMatch(/^Weekly limit 86% · resets 3:40/);
    const hit = limitWarning([{ name: 'five_hour', utilization: 1, resetsAt: later }], now)!;
    expect(hit).toMatchObject({ reached: true });
    expect(hit.text).toMatch(/^Rate limited until 3:40.* \(5-hour limit\)$/);
    // A window that already reset says nothing.
    expect(limitWarning([{ name: 'five_hour', utilization: 1, resetsAt: now - 1 }], now)).toBeNull();
  });
});
