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
  // PDT-503: stopping work, issues, documents and the PM's notes.
  'stop_ticket',
  'issue_create',
  'report_bug',
  'link_issue',
  'unlink_issue',
  'document_create',
  'doc_edit',
  'sheet_edit',
  'document_link',
  'save_note',
  'delete_note',
]);

/** A call that shows as a card of its own (never folded with the turn's other tools). */
export const isCardTool = (tool: string) => CARD_TOOLS.has(pdtTool(tool) ?? '');

/**
 * The text of a pdt-mcp result's field without the block that marks it as outside text for the agent
 * (`<untrusted id="…">` … `</untrusted id="…">`, crates/pdt-core/src/fence.rs), its escaped tags written back; any
 * other value as it is.
 */
export function unfenced(v: unknown): unknown {
  if (typeof v !== 'string') return v;
  const m = /^<untrusted id="([A-Za-z0-9]*)">\n([\s\S]*)\n<\/untrusted id="\1">$/.exec(v);
  return m ? m[2].replace(/&lt;(?=\s*\/?\s*untrusted)/gi, '<') : v;
}

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

/**
 * "it finishes, fails or needs you", "someone comments", "it finishes or an agent reports": what a watch tells about
 * (default finished, failed and needs_you), as a clause with its subject, to follow "when".
 */
export function watchEventsText(events: string[]): string {
  // What the target does reads after one "it"; what others do names who.
  const ofIt: Record<string, string> = { finished: 'finishes', failed: 'fails', needs_you: 'needs you' };
  const ofOthers: Record<string, string> = { outcome: 'an agent reports', comment: 'someone comments' };
  const given = events.length ? events : ['finished', 'failed', 'needs_you'];
  const own = given.filter((e) => !(e in ofOthers)).map((e) => ofIt[e] ?? e);
  const list = [...own.map((w, i) => (i === 0 ? `it ${w}` : w)), ...given.filter((e) => e in ofOthers).map((e) => ofOthers[e])];
  return list.length > 1 ? `${list.slice(0, -1).join(', ')} or ${list[list.length - 1]}` : list[0];
}

/** A `stop_ticket` call: the ticket and, once it ran, where it went and what stopped. */
export interface StoppedTicket {
  key: string;
  stage: string | null;
  runsCancelled: number;
  sessionsStopped: number;
  worktreeRemoved: boolean;
  branchNote: string | null;
}

const count = (v: unknown) => (typeof v === 'number' ? v : Array.isArray(v) ? v.length : 0);

export function stoppedTicket(input: unknown, output?: string): StoppedTicket {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  return {
    key: nonBlank(r.key) ?? nonBlank(a.ticket) ?? 'a ticket',
    stage: nonBlank(r.stage) ?? nonBlank(a.stage),
    runsCancelled: count(r.runsCancelled),
    sessionsStopped: count(r.sessionsStopped),
    worktreeRemoved: r.worktreeRemoved === true,
    branchNote: nonBlank(r.branchNote),
  };
}

/** `owner/repo#12` from a GitHub issue's address, else null. */
export function issueKeyOf(url: string | null): string | null {
  const m = url ? /^https?:\/\/[^/]+\/([^/]+\/[^/]+)\/issues\/(\d+)/.exec(url) : null;
  return m ? `${m[1]}#${m[2]}` : null;
}

/** An issue an `issue_create` / `report_bug` call files (or a local ticket a bug report becomes). */
export interface FiledIssueView {
  title: string | null;
  /** `owner/repo#12`. */
  issue: string | null;
  url: string | null;
  /** The ticket that links it (`issue_create` from a ticket), or the ticket a bug report became. */
  ticket: string | null;
}

export function filedIssue(input: unknown, output?: string): FiledIssueView {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  // report_bug: an issue on PDT's repository ({kind: issue, number, url}) or a local ticket ({kind: ticket, key}).
  if (r.kind === 'ticket') return { title: nonBlank(a.title), issue: null, url: null, ticket: nonBlank(r.key) };
  const url = nonBlank(r.url);
  return {
    title: nonBlank(r.title) ?? nonBlank(a.title),
    issue: nonBlank(r.issue) ?? issueKeyOf(url),
    url,
    ticket: nonBlank(r.ticketKey) ?? nonBlank(a.ticket),
  };
}

/** A `link_issue` / `unlink_issue` call: the ticket and the issue (or plain link) linked or removed. */
export function issueLink(input: unknown, output?: string): { ticket: string; key: string | null; url: string | null; title: string | null } {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  const i = isRecord(r.issue) ? r.issue : {};
  const t = isRecord(r.ticket) ? r.ticket : {};
  const given = nonBlank(a.issue);
  const isUrl = !!given && /^https?:\/\//.test(given);
  const url = nonBlank(i.url) ?? (isUrl ? given : null);
  return {
    ticket: nonBlank(t.key) ?? nonBlank(a.ticket) ?? 'a ticket',
    key: nonBlank(i.key) ?? (isUrl ? issueKeyOf(given) : given),
    url,
    title: nonBlank(unfenced(i.title)) ?? nonBlank(a.label),
  };
}

/** A document a `document_create`, `doc_edit`, `sheet_edit` or `document_link` call made, changed or linked. */
export interface DocumentView {
  id: string | null;
  title: string;
  kind: 'doc' | 'sheet' | null;
  url: string | null;
  projects: string[];
  tickets: string[];
  /** What the call did to it, in a line. */
  change: string | null;
  openedBeside: boolean;
}

const quoted = (v: unknown) => `\u201c${nonBlank(v) ?? '?'}\u201d`;

/** One edit (`DocumentEdit`) as a phrase: "replaced the section “Scope”". */
function editText(e: unknown): string {
  const x = isRecord(e) ? e : {};
  const table = nonBlank(x.heading) ? ` under ${quoted(x.heading)}` : '';
  switch (x.op) {
    case 'replace_text': return 'rewrote the whole document';
    case 'append_text': return 'added text at the end';
    case 'replace_section': return `replaced the section ${quoted(x.heading)}`;
    case 'insert_after_heading': return `added text under ${quoted(x.heading)}`;
    case 'replace_string': return `replaced ${quoted(x.find)}${x.all === true ? ' everywhere' : ''}`;
    case 'table_append_row': return `added a table row${table}`;
    case 'table_set_cell': return `changed a table cell${table}`;
    case 'set_cells': return `set cells from ${nonBlank(x.cell) ?? '?'} in ${nonBlank(x.sheet) ?? 'a sheet'}`;
    case 'add_sheet': return `added the sheet ${quoted(x.name)}`;
    case 'rename_sheet': return `renamed ${quoted(x.sheet)} to ${quoted(x.name)}`;
    case 'delete_sheet': return `deleted the sheet ${quoted(x.sheet)}`;
    case 'format_range': return `formatted ${nonBlank(x.range) ?? 'a range'}`;
    case 'conditional_format': return `added a conditional format on ${nonBlank(x.range) ?? 'a range'}`;
    case 'add_chart': return 'added a chart';
    case 'update_chart': return 'changed a chart';
    case 'delete_chart': return 'deleted a chart';
    case 'sort': return `sorted ${nonBlank(x.range) ?? 'a range'}`;
    case 'filter': return `filtered ${nonBlank(x.range) ?? 'a range'}`;
    case 'clear_filter': return 'cleared a filter';
    case 'set_borders': return `set borders on ${nonBlank(x.range) ?? 'a range'}`;
    case 'column_width': return `resized column ${nonBlank(x.columns) ?? '?'}`;
    case 'freeze': return 'froze rows or columns';
    case 'validate': return `added validation on ${nonBlank(x.range) ?? 'a range'}`;
    default: return typeof x.op === 'string' ? x.op.replace(/_/g, ' ') : 'changed it';
  }
}

/** A list of edits in a line: "Replaced the section “Scope”, added a table row and 2 more". */
export function editsText(edits: unknown): string | null {
  const list = Array.isArray(edits) ? edits.map(editText) : [];
  if (!list.length) return null;
  const shown = list.length > 3 ? [...list.slice(0, 2), `${list.length - 2} more`] : list;
  const text = shown.length > 1 ? `${shown.slice(0, -1).join(', ')} and ${shown[shown.length - 1]}` : shown[0];
  return text[0].toUpperCase() + text.slice(1);
}

export function documentOf(tool: string, input: unknown, output?: string): DocumentView {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  const name = pdtTool(tool);
  const kind = r.kind === 'doc' || r.kind === 'sheet' ? r.kind
    : name === 'doc_edit' ? 'doc' : name === 'sheet_edit' ? 'sheet'
      : a.kind === 'doc' || a.kind === 'sheet' ? a.kind : null;
  // Made, linked or edited: the document is in the result once it ran (an id, or its title as the call named it).
  const given = nonBlank(a.document);
  const target = nonBlank(a.ticket) ?? nonBlank(a.project);
  const change = name === 'doc_edit' || name === 'sheet_edit' ? editsText(a.edits)
    // A new link shows in the card's links; one removed is said.
    : name === 'document_link' && target && a.unlink === true ? `No longer linked to ${target}`
      : null;
  return {
    id: nonBlank(r.id) ?? (name === 'document_create' ? null : given),
    title: nonBlank(unfenced(r.title)) ?? nonBlank(a.title) ?? given ?? 'a document',
    kind,
    url: nonBlank(r.url),
    projects: strings(r.projects ?? (name === 'document_create' && a.project ? [a.project] : [])),
    tickets: strings(r.tickets ?? (name === 'document_create' && a.ticket ? [a.ticket] : [])),
    change,
    openedBeside: r.openBeside === true,
  };
}

/** A `save_note` / `delete_note` call's note. */
export interface NoteView {
  id: string | null;
  title: string | null;
  body: string | null;
  /** Its project's key; null when host-wide (or not known). */
  project: string | null;
  pinned: boolean;
  /** `save_note` with an id: it replaced a note. */
  replaced: boolean;
}

export function noteOf(input: unknown, output?: string): NoteView {
  const a = isRecord(input) ? input : {};
  const r = resultOf(output) ?? {};
  return {
    id: nonBlank(r.id) ?? nonBlank(a.id),
    title: nonBlank(r.title) ?? nonBlank(a.title),
    body: nonBlank(a.body),
    project: nonBlank(r.project) ?? nonBlank(a.project),
    pinned: (r.pinned ?? a.pinned) === true,
    replaced: !!nonBlank(a.id),
  };
}

/** The lookups whose folded rows name the tickets they read (PDT-503). */
const LOOKUP_TOOLS = new Set(['get_ticket', 'search_tickets', 'get_overview']);

const isLookupTool = (tool: string) => LOOKUP_TOOLS.has(pdtTool(tool) ?? '');

const keysIn = (v: unknown) => (Array.isArray(v) ? v.flatMap((t) => (isRecord(t) && typeof t.key === 'string' ? [t.key] : [])) : []);

/**
 * The tickets a lookup read: `get_ticket`'s ticket, `search_tickets`' matches, `get_overview`'s tickets in progress.
 * Empty for any other tool, or one that hasn't answered.
 */
function lookupKeys(tool: string, input: unknown, output?: string): string[] {
  const r = resultOf(output);
  switch (pdtTool(tool)) {
    case 'get_ticket': {
      const a = isRecord(input) ? input : {};
      const key = nonBlank(r?.key) ?? nonBlank(a.ticket);
      return key ? [key] : [];
    }
    case 'search_tickets':
      return keysIn(r?.tickets);
    case 'get_overview':
      return keysIn(r?.inProgress);
    default:
      return [];
  }
}

/**
 * The tickets a run of folded calls read, each once, when every call is a lookup (`get_ticket`, `search_tickets`,
 * `get_overview`) and they read at least one; else null (the row keeps its summary).
 */
export function lookedUp(calls: { tool: string; input: unknown; output?: string }[]): string[] | null {
  if (!calls.length || !calls.every((c) => isLookupTool(c.tool))) return null;
  const keys = [...new Set(calls.flatMap((c) => lookupKeys(c.tool, c.input, c.output)))];
  return keys.length ? keys : null;
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
