import type { InboxGroup } from '../api/gen/InboxGroup';
import type { InboxItem } from '../api/gen/InboxItem';
import type { InboxKind } from '../api/gen/InboxKind';
import { routePath } from '../shell/route';

/** The Inbox's groups, most urgent first (UX spec §8.4), as the API sorts them. */
export const GROUPS: { id: InboxGroup; title: string }[] = [
  { id: 'question', title: 'Questions' },
  { id: 'approval', title: 'Approvals' },
  { id: 'session', title: 'Sessions waiting' },
  { id: 'stalled', title: 'Stalled' },
  { id: 'loop_limit', title: 'Loop limit' },
  { id: 'notice', title: 'Unheard notices' },
  { id: 'admit', title: 'Triaged tickets' },
  { id: 'proposal', title: 'Workflow proposals' },
  { id: 'gate', title: 'Gates' },
  { id: 'accept', title: 'Ready to accept' },
];

export const KIND_LABEL: Record<InboxKind, string> = {
  question: 'Question', approval: 'Approval', stalled: 'Stalled', loop_limit: 'Loop limit', gate: 'Gate', manual: 'Your turn',
  session: 'Session waiting',
  timed_out: 'Timed out',
  notice: 'Unheard notice',
  secret: 'Secret request',
  connect: 'Connect',
  admit: 'Triaged ticket',
  confirm: 'Agent call',
  proposal: 'Workflow proposal',
  mcp_sign_in: 'Sign in again',
  storage: "Can't save",
  low_disk: 'Low disk space',
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

/** A stalled step whose first message never reached its session: it offers Resume with this message (PDT-144). */
export function resends(item: InboxItem): boolean {
  return item.kind === 'stalled' && item.choices.some((c) => c.id === 'resend');
}

/** Whether `e` dismisses it (questions and stalled steps, whose step keeps waiting; unheard notices). */
export function dismissable(item: InboxItem): boolean {
  return item.kind === 'question' || item.kind === 'secret' || item.kind === 'stalled' || item.kind === 'notice';
}

/** Where "Connect GitHub" (PDT-177) sends the user: GitHub's plugin page, Settings › Plugins › GitHub (PDT-228). */
export const CONNECT_PATH = '/settings/plugins/github';

/** Where "Sign in to <server> again" (an MCP server's OAuth sign-in, PDT-322) sends the user. */
export const MCP_SERVERS_PATH = '/settings/mcp-servers';

/** What an unheard notice said, and its settling, in a line (PDT-139). */
export function noticeText(item: InboxItem): string {
  return typeof item.body.text === 'string' ? item.body.text : item.title;
}

/** A gate's whole question (Markdown) when it says more than the item's title, which is one line (PDT-306). */
export function gateQuestion(item: InboxItem): string | null {
  const q = item.body.question;
  return item.kind === 'gate' && typeof q === 'string' && q.trim() && q.trim() !== item.title.trim() ? q : null;
}

/** The button that opens the pull request a gate is about, in its forge's words: "Open merge request" on GitLab. */
export function changeRequestLabel(item: InboxItem): string | null {
  return item.changeRequest ? `Open ${item.changeRequest.name}` : null;
}

/** A triaged ticket's notes, the issue it came from and whether it came from outside PDT (PDT-225). */
export function triageOf(item: InboxItem): { summary: string | null; issue: string | null; external: boolean } {
  const b = item.body;
  return {
    summary: typeof b.summary === 'string' ? b.summary : null,
    issue: typeof b.issue === 'string' ? b.issue : null,
    external: b.external === true,
  };
}

/** An asking tool call's tool, agent, session and arguments (pretty JSON) (PDT-256). */
export function confirmOf(item: InboxItem): { tool: string; agent: string | null; session: string | null; input: string } {
  const b = item.body;
  return {
    tool: typeof b.tool === 'string' ? b.tool : item.title,
    agent: typeof b.agent === 'string' ? b.agent : null,
    session: typeof b.sessionTitle === 'string' && b.sessionTitle ? b.sessionTitle : null,
    input: JSON.stringify(b.input ?? {}, null, 2),
  };
}

/** A workflow version an agent proposed (PDT-320): the workflow, the version, who proposed it, their note and what changed. */
export function proposalOf(item: InboxItem): { workflowId: string; workflow: string; version: number; note: string; summary: string } {
  const b = item.body;
  return {
    workflowId: typeof b.workflowId === 'string' ? b.workflowId : '',
    workflow: typeof b.workflow === 'string' ? b.workflow : '',
    version: typeof b.version === 'number' ? b.version : 0,
    note: typeof b.note === 'string' ? b.note : '',
    summary: typeof b.summary === 'string' ? b.summary : '',
  };
}

/** Where a proposal's Open goes: its workflow's Versions tab, which shows the whole change. */
export function proposalPath(item: InboxItem): string {
  return routePath({ view: 'workflows', target: { kind: 'workflow', id: proposalOf(item).workflowId, tab: 'versions' } });
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
