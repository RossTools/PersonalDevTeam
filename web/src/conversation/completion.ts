import type { AgentCommandInfo } from '../api/gen/AgentCommandInfo';
import type { RateLimitWindow } from '../api/gen/RateLimitWindow';
import type { Suggestion } from '@ui';

/** What the word at the caret asks to complete: a `/command` (only as the message's first word) or an `@file`. */
export interface Completion {
  kind: 'command' | 'file';
  /** Where the word starts (its `/` or `@`). */
  start: number;
  /** What is typed after the `/` or `@`, up to the caret. */
  query: string;
}

export function completionAt(text: string, caret: number): Completion | null {
  const before = text.slice(0, caret);
  const cmd = /^\/(\S*)$/.exec(before);
  if (cmd) return { kind: 'command', start: 0, query: cmd[1] };
  const file = /(^|\s)@([^\s@]*)$/.exec(before);
  if (file) return { kind: 'file', start: before.length - file[2].length - 1, query: file[2] };
  return null;
}

/** Replaces the word being completed (through any rest of it after the caret) with `insert` and a space. */
export function applyCompletion(text: string, caret: number, c: Completion, insert: string): { text: string; caret: number } {
  let end = caret;
  while (end < text.length && !/\s/.test(text[end])) end++;
  const after = text.slice(end).replace(/^\s/, '');
  const head = `${text.slice(0, c.start)}${insert} `;
  return { text: head + after, caret: head.length };
}

/** The agent's commands matching a `/` query (prefix matches first), with TUI-only ones shown but not pickable. */
export function commandSuggestions(commands: AgentCommandInfo[], terminalOnly: string[], query: string): Suggestion[] {
  const q = query.toLowerCase();
  const seen = new Set<string>();
  const all: { name: string; detail?: string; terminal: boolean }[] = [];
  for (const c of commands) {
    if (seen.has(c.name)) continue;
    seen.add(c.name);
    const hint = c.argumentHint ? `${c.argumentHint} ` : '';
    all.push({ name: c.name, detail: `${hint}${c.description}`.trim() || undefined, terminal: terminalOnly.includes(c.name) });
  }
  for (const name of terminalOnly) if (!seen.has(name)) all.push({ name, terminal: true });
  const starts = all.filter((c) => c.name.toLowerCase().startsWith(q));
  const contains = all.filter((c) => !c.name.toLowerCase().startsWith(q) && c.name.toLowerCase().includes(q));
  return [...starts, ...contains].slice(0, 50).map((c) => ({
    id: c.name,
    label: `/${c.name}`,
    ...(c.detail ? { detail: c.detail } : {}),
    ...(c.terminal ? { disabled: true, hint: 'Only in the terminal' } : {}),
  }));
}

/** `five_hour` / `300_minutes` → "5-hour"; `seven_day` / `10080_minutes` → "weekly". */
export function windowLabel(name: string): string {
  const minutes = /^(\d+)_minutes$/.exec(name)?.[1];
  const m = minutes ? Number(minutes) : name === 'five_hour' ? 300 : name === 'seven_day' ? 10_080 : null;
  if (m === null) return name.replace(/_/g, ' ');
  if (m === 10_080) return 'weekly';
  if (m % 1440 === 0) return `${m / 1440}-day`;
  if (m % 60 === 0) return `${m / 60}-hour`;
  return `${m}-minute`;
}

export interface LimitWarning {
  /** "Weekly limit 86% · resets Tue 3:40 PM", "Rate limited until 3:40 PM". */
  text: string;
  /** At the limit: nothing more will run until it resets. */
  reached: boolean;
}

const at = (ms: number, now: number) => {
  const d = new Date(ms);
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return d.toLocaleString(undefined, sameDay ? { hour: 'numeric', minute: '2-digit' } : { weekday: 'short', hour: 'numeric', minute: '2-digit' });
};

/** The rate-limit window closest to its limit, once it is at 80 % or more. */
export function limitWarning(windows: RateLimitWindow[], now: number): LimitWarning | null {
  const live = windows.filter((w) => w.resetsAt == null || w.resetsAt > now);
  const worst = live.reduce<RateLimitWindow | null>((a, w) => (!a || w.utilization > a.utilization ? w : a), null);
  if (!worst || worst.utilization < 0.8) return null;
  const label = windowLabel(worst.name);
  const reset = worst.resetsAt != null ? at(worst.resetsAt, now) : null;
  if (worst.utilization >= 1) {
    return { text: reset ? `Rate limited until ${reset} (${label} limit)` : `Rate limited (${label} limit)`, reached: true };
  }
  const pct = Math.round(worst.utilization * 100);
  return { text: `${label[0].toUpperCase()}${label.slice(1)} limit ${pct}%${reset ? ` · resets ${reset}` : ''}`, reached: false };
}
