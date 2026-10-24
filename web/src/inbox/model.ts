import type { InboxGroup } from '../api/gen/InboxGroup';
import type { InboxItem } from '../api/gen/InboxItem';
import type { InboxKind } from '../api/gen/InboxKind';

/** The Inbox's groups, most urgent first (UX spec §8.4), as the API sorts them. */
export const GROUPS: { id: InboxGroup; title: string }[] = [
  { id: 'question', title: 'Questions' },
  { id: 'approval', title: 'Approvals' },
  { id: 'session', title: 'Sessions waiting' },
  { id: 'stalled', title: 'Stalled' },
  { id: 'loop_limit', title: 'Loop limit' },
  { id: 'notice', title: 'Unheard notices' },
  { id: 'gate', title: 'Gates' },
  { id: 'accept', title: 'Ready to accept' },
];

export const KIND_LABEL: Record<InboxKind, string> = {
  question: 'Question', approval: 'Approval', stalled: 'Stalled', loop_limit: 'Loop limit', gate: 'Gate', manual: 'Your turn',
  session: 'Session waiting',
  timed_out: 'Timed out',
  notice: 'Unheard notice',
};

/** The items of each group that has any, in the API's order. */
export function grouped(items: InboxItem[]): { id: InboxGroup; title: string; items: InboxItem[] }[] {
  return GROUPS.map((g) => ({ ...g, items: items.filter((i) => i.group === g.id) })).filter((g) => g.items.length > 0);
}

/** The item `by` places after (or before) `id` in list order; the first one when `id` is gone. */
export function step(items: InboxItem[], id: string | null, by: 1 | -1): string | null {
  const order = grouped(items).flatMap((g) => g.items);
  if (order.length === 0) return null;
  const at = order.findIndex((i) => i.id === id);
  if (at < 0) return order[0].id;
  return order[Math.min(order.length - 1, Math.max(0, at + by))].id;
}

/** What to show after `id` was answered: the next item, else the one before, else none. */
export function after(items: InboxItem[], id: string): string | null {
  const order = grouped(items).flatMap((g) => g.items);
  const at = order.findIndex((i) => i.id === id);
  const rest = order.filter((i) => i.id !== id);
  if (rest.length === 0) return null;
  return (rest[Math.min(Math.max(at, 0), rest.length - 1)] ?? rest[0]).id;
}

/** An agent's questions (`ask_user`). */
export interface InboxQuestion { question: string; choices: string[] }

export function questionsOf(item: InboxItem): InboxQuestion[] {
  const qs = item.body.questions;
  if (!Array.isArray(qs)) return [];
  return qs.flatMap((q) => {
    if (typeof q !== 'object' || q === null) return [];
    const r = q as Record<string, unknown>;
    if (typeof r.question !== 'string') return [];
    return [{ question: r.question, choices: Array.isArray(r.choices) ? r.choices.filter((c): c is string => typeof c === 'string') : [] }];
  });
}

/** An earlier pass of a looping step. */
export interface Pass { persona: string; outcome: string | null; summary: string | null }

export function passesOf(item: InboxItem): Pass[] {
  const ps = item.body.passes;
  if (!Array.isArray(ps)) return [];
  return ps.map((p) => {
    const r = (p ?? {}) as Record<string, unknown>;
    return {
      persona: typeof r.persona === 'string' ? r.persona : 'Agent',
      outcome: typeof r.outcome === 'string' ? r.outcome : null,
      summary: typeof r.summary === 'string' ? r.summary : null,
    };
  });
}

/** The item's primary choice (Enter takes it). */
export function primary(item: InboxItem): string | null {
  return item.choices.find((c) => c.primary)?.id ?? null;
}

/** Whether `e` dismisses it (questions and stalled steps, whose step keeps waiting; unheard notices). */
export function dismissable(item: InboxItem): boolean {
  return item.kind === 'question' || item.kind === 'stalled' || item.kind === 'notice';
}

/** What an unheard notice said, and its settling, in a line (PDT-139). */
export function noticeText(item: InboxItem): string {
  return typeof item.body.text === 'string' ? item.body.text : item.title;
}

/** An Update's id in the Inbox's selection (Needs-you items use their own ids). */
export const UPDATE = 'update:';

export function updateSelection(id: string): string {
  return `${UPDATE}${id}`;
}

/** The notification an Inbox selection names, if it is an Update. */
export function selectedUpdate(selection: string | null): string | null {
  return selection?.startsWith(UPDATE) ? selection.slice(UPDATE.length) : null;
}

/** An item's name on its own screen: its kind, or Accept for a gate under Ready to accept. */
export function itemLabel(item: InboxItem): string {
  return item.group === 'accept' ? 'Accept' : KIND_LABEL[item.kind];
}

/** Where `id` stands in list order: "Approval · 2 of 4" (phones, PDT-117). */
export function triageTitle(items: InboxItem[], id: string): string | null {
  const order = grouped(items).flatMap((g) => g.items);
  const at = order.findIndex((i) => i.id === id);
  return at < 0 ? null : `${itemLabel(order[at])} · ${at + 1} of ${order.length}`;
}

/** A choice that merges the ticket's work: Merge, and the primary choice of a gate under Ready to accept. */
export function merges(item: InboxItem, choice: string): boolean {
  if (item.kind !== 'gate') return false;
  if (/^merge\b/i.test(choice)) return true;
  return item.group === 'accept' && item.choices.find((c) => c.id === choice)?.primary === true;
}
