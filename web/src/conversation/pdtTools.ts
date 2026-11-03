import { isRecord, nonBlank } from '../common/guards';
// PDT's own tools in a conversation (PDT-382): which calls show as cards instead of folding into "Used N tools", and
// what each card reads from the call's input and result.

const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);

/** A pdt-mcp tool's own name: Claude's (and ACP agents') `mcp__pdt__create_ticket`, Codex's `mcp:pdt/create_ticket`. */
export function pdtTool(tool: string): string | null {
  return /^mcp__pdt__(.+)$/.exec(tool)?.[1] ?? /^mcp:pdt\/(.+)$/.exec(tool)?.[1] ?? null;
}

/** The PDT tools a conversation shows as cards. */
const CARD_TOOLS = new Set([
  'create_ticket',
  'edit_ticket',
  'link_tickets',
  'unlink_tickets',
  'send_to_session',
  'add_ticket_comment',
  'workflow_run',
  'watch',
  'unwatch',
]);

/** A call that shows as a card of its own (never folded with the turn's other tools). */
export const isCardTool = (tool: string) => CARD_TOOLS.has(pdtTool(tool) ?? '');

/**
 * A pdt-mcp result as an object: the JSON text Claude passes on, or Codex's whole MCP result (its
 * `structuredContent`, else its text). Null for an error or anything else.
 */
export function resultOf(output: string | undefined): Record<string, unknown> | null {
  if (!output) return null;
  let v: unknown;
  try {
    v = JSON.parse(output);
  } catch {
    return null;
  }
  if (!isRecord(v)) return null;
  if (isRecord(v.structuredContent)) return v.structuredContent;
  if (Array.isArray(v.content)) {
    const text = v.content.find((c): c is { text: string } => isRecord(c) && typeof c.text === 'string');
    return text ? resultOf(text.text) : null;
  }
  return v;
}

/** A ticket a `create_ticket` call makes (its key once made). */
export interface CreatedTicket {
  key: string | null;
  title: string;
  stage: string;
  priority: string | null;
  labels: string[];
  description: string | null;
  /** It waits in the Inbox as a proposal. */
  proposal: boolean;
}

export function createdTicket(input: unknown, output?: string): CreatedTicket {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  return {
    key: nonBlank(r.key),
    title: nonBlank(a.title) ?? 'Untitled',
    stage: nonBlank(r.stage) ?? nonBlank(a.stage) ?? 'draft',
    priority: nonBlank(a.priority),
    labels: strings(a.labels),
    description: nonBlank(a.description),
    proposal: r.proposal === true,
  };
}

/** One field an edit changes: `from` is known once it ran (pdt-mcp says), `to` from the call. */
export interface FieldChange {
  field: string;
  from?: unknown;
  to?: unknown;
}

const EDIT_FIELDS = ['title', 'stage', 'priority', 'labels', 'areas', 'parent', 'description'] as const;

/** What an `edit_ticket` call changes: pdt-mcp's list once it ran, else the fields the call gives. */
export function ticketEdit(input: unknown, output?: string): { key: string; changes: FieldChange[]; files: number } {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  const done = Array.isArray(r.changes)
    ? r.changes.flatMap((c): FieldChange[] => (isRecord(c) && typeof c.field === 'string' ? [{ field: c.field, from: c.from, to: c.to }] : []))
    : null;
  const asked = EDIT_FIELDS.flatMap((f): FieldChange[] => (a[f] !== undefined ? [{ field: f, to: f === 'description' ? undefined : a[f] }] : []));
  return { key: nonBlank(r.key) ?? nonBlank(a.ticket) ?? 'a ticket', changes: done ?? asked, files: strings(a.files).length };
}

export type LinkKind = 'blocks' | 'blockedBy' | 'relates';

/** A `link_tickets` / `unlink_tickets` call: "PDT-2 blocked by PDT-1". */
export function ticketLink(input: unknown): { ticket: string; other: string; kind: LinkKind } {
  const a = isRecord(input) ? input : {};
  const kind = a.kind === 'blocks' || a.kind === 'blockedBy' ? a.kind : 'relates';
  return { ticket: nonBlank(a.ticket) ?? '?', other: nonBlank(a.other) ?? '?', kind };
}

export const LINK_TEXT: Record<LinkKind, string> = { blocks: 'blocks', blockedBy: 'blocked by', relates: 'relates to' };

/** A `send_to_session` call: the message and who gets it (pdt-mcp names them once it ran). */
export function sentMessage(input: unknown, output?: string): { text: string; to: string | null; ticket: string | null; session: string | null } {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  return { text: nonBlank(a.text) ?? '', to: nonBlank(r.to), ticket: nonBlank(r.ticket), session: nonBlank(a.session) };
}

/** An `add_ticket_comment` call: the comment and its ticket (the session's own when the call names none). */
export function ticketComment(input: unknown, output?: string): { key: string | null; body: string; files: number } {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  return { key: nonBlank(r.key) ?? nonBlank(a.ticket), body: nonBlank(a.body) ?? '', files: strings(a.files).length };
}

/** A `workflow_run` call: the workflow and, once started, its run. */
export function workflowRun(input: unknown, output?: string): { workflow: string; workflowId: string | null; runId: string | null; project: string | null } {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  return {
    workflow: nonBlank(r.workflow) ?? nonBlank(a.name) ?? nonBlank(a.id) ?? 'a workflow',
    workflowId: nonBlank(a.id),
    runId: nonBlank(r.runId),
    project: nonBlank(a.project),
  };
}

/** A `watch` call's watch (once made), or what it asked to watch. */
export interface WatchView {
  id: string | null;
  label: string;
  /** `ticket` or `run`. */
  target: 'ticket' | 'run';
  events: string[];
}

export function watchOf(input: unknown, output?: string): WatchView {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  const w = Array.isArray(r.watching) && isRecord(r.watching[0]) ? r.watching[0] : null;
  const target = (w?.target ?? (a.run ? 'run' : 'ticket')) === 'run' ? 'run' : 'ticket';
  return {
    id: nonBlank(w?.id),
    label: nonBlank(w?.targetLabel) ?? nonBlank(a.ticket) ?? nonBlank(a.run) ?? '?',
    target,
    events: strings(w?.events ?? a.events),
  };
}

/** "finishes, fails or needs you": what a watch tells about (default finished, failed and needs_you). */
export function watchEventsText(events: string[]): string {
  const words: Record<string, string> = {
    finished: 'finishes',
    failed: 'fails',
    needs_you: 'needs you',
    outcome: 'an agent reports',
    comment: 'someone comments',
  };
  const list = (events.length ? events : ['finished', 'failed', 'needs_you']).map((e) => words[e] ?? e);
  return list.length > 1 ? `${list.slice(0, -1).join(', ')} or ${list[list.length - 1]}` : list[0];
}

/**
 * The approval card's question for a PDT tool that waits for the user's yes ("Create this ticket?"), or null for any
 * other tool.
 */
export function pdtApprovalTitle(tool: string, input: unknown): string | null {
  const a = isRecord(input) ? input : {};
  switch (pdtTool(tool)) {
    case 'create_ticket':
      return 'Create this ticket?';
    case 'edit_ticket':
      return `Change ${nonBlank(a.ticket) ?? 'this ticket'}?`;
    case 'link_tickets':
      return 'Link these tickets?';
    case 'unlink_tickets':
      return 'Remove this link?';
    case 'send_to_session':
      return 'Send this message?';
    case 'workflow_run':
      return 'Run this workflow?';
    case 'stop_ticket':
      return `Stop the work on ${nonBlank(a.ticket) ?? 'this ticket'}?`;
    default:
      return null;
  }
}
