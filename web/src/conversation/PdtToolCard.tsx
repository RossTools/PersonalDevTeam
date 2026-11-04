// PDT's own tool calls in a conversation as cards (PDT-382): what the agent created, changed, linked, sent, ran or
// watched, instead of a folded "Used N tools" line.
import { useContext, type ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  ArrowRight, Bug, CircleDot, CircleStop, Eye, FileText, GitFork, Link2, Mail, MessageSquare, PanelRight, Pencil, Pin, Play, Plus, Sheet,
  StickyNote, Trash2,
} from 'lucide-react';
import { Badge, Button, Disclosure, DiffStat, diffStats, Spinner } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { Priority } from '../api/gen/Priority';
import type { TicketStage } from '../api/gen/TicketStage';
import { Markdown } from '../common/Markdown';
import { plural } from '../common/text';
import { PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { TicketPill } from '../common/TicketPill';
import { actionError, useAction } from '../common/useAction';
import { MarkdownRefsContext } from '../common/markdownRefs';
import { besideKey } from '../documents/BesidePanel';
import { routePath } from '../shell/route';
import { RUN_STATUS } from '../workflows/runs';
import {
  createdTicket,
  documentOf,
  filedIssue,
  issueLink,
  noteOf,
  stoppedTicket,
  LINK_TEXT,
  pdtTool,
  resultOf,
  sentMessage,
  ticketComment,
  ticketEdit,
  ticketLink,
  watchEventsText,
  watchOf,
  workflowRun,
  type CreatedTicket,
  type FieldChange,
} from './pdtTools';
import { workflowKeys } from '../workflows/queries';
import { POLL } from '../common/poll';

/** A PDT tool call as the card needs it (a `tool_call` event's fields). */
export interface PdtCall {
  tool: string;
  input: unknown;
  status: 'running' | 'done' | 'failed' | 'declined';
  output?: string;
}

export const stageLabel = (s: string) => STAGE_LABEL[s as TicketStage] ?? s;
export const priorityLabel = (p: string) => PRIORITY_LABEL[p as Priority] ?? p;

/** A key as a pill when it looks like one (`PDT-12`), else as written (an id). */
export function Key({ value }: { value: string | null }) {
  if (!value) return <span className="text-muted-foreground">a ticket</span>;
  return /^[A-Za-z][A-Za-z0-9]*-\d+$/.test(value) ? <TicketPill ticketKey={value.toUpperCase()} /> : <span className="font-mono text-meta">{value}</span>;
}

/** The most ticket pills a folded row shows; the rest are counted. */
const PILLS_SHOWN = 6;

/** Ticket keys as pills ("Read PDT-499, PDT-492"), for a folded lookup's row: the first few, then "+3". */
export function TicketPills({ keys }: { keys: string[] }) {
  const rest = keys.slice(PILLS_SHOWN);
  return (
    <>
      {keys.slice(0, PILLS_SHOWN).map((k) => <Key key={k} value={k} />)}
      {rest.length > 0 && <span className="text-meta text-muted-foreground" title={rest.join(', ')}>+{rest.length}</span>}
    </>
  );
}

/** The frame every card shares: an icon and a heading, something on the right, then its rows. */
function Card({ icon, heading, aside, children, label }: {
  icon: ReactNode;
  heading: ReactNode;
  aside?: ReactNode;
  children?: ReactNode;
  /** The card's accessible name (the heading's text). */
  label: string;
}) {
  return (
    <section aria-label={label} data-slot="pdt-card" className="rounded-lg border border-border bg-card text-sm shadow-card">
      <header className={cn('flex min-h-9 items-center gap-2 px-3 py-1.5 text-xs text-muted-foreground', children && 'border-b border-border')}>
        <span aria-hidden className="grid size-4 shrink-0 place-items-center text-link [&>svg]:size-3.5">{icon}</span>
        <span className="flex min-w-0 flex-wrap items-center gap-1.5">{heading}</span>
        <span className="flex-1" />
        {aside}
      </header>
      {children && <div className="flex flex-col divide-y divide-border px-3">{children}</div>}
    </section>
  );
}

/** What a call that hasn't (or didn't) run shows instead of its result. */
function callState(call: PdtCall): { aside: ReactNode; error: string | null } | null {
  if (call.status === 'running') return { aside: <Spinner decorative className="size-3.5" />, error: null };
  if (call.status === 'declined') return { aside: <span>Declined</span>, error: null };
  if (call.status === 'failed') return { aside: <span className="text-failed">Failed</span>, error: call.output?.trim() || null };
  return null;
}

function Failure({ text }: { text: string | null }) {
  return text ? <p className="py-2 text-xs break-words text-failed">{text}</p> : null;
}

function TicketRow({ t }: { t: CreatedTicket }) {
  return (
    <div className="flex flex-wrap items-center gap-2 py-2">
      {t.key && <Key value={t.key} />}
      <span className="min-w-0 flex-1 font-medium">{t.title}</span>
      <span className="flex flex-wrap items-center gap-1">
        <Badge>{stageLabel(t.stage)}</Badge>
        {t.priority && t.priority !== 'none' && <Badge>{priorityLabel(t.priority)}</Badge>}
        {t.labels.map((l) => <Badge key={l} tone="accent">{l}</Badge>)}
      </span>
    </div>
  );
}

/** One or more `create_ticket` calls in a row: "Created 2 tickets", a row each. */
export function CreateTicketsCard({ calls }: { calls: PdtCall[] }) {
  const made = calls.filter((c) => c.status === 'done');
  const tickets = made.map((c) => createdTicket(c.input, c.output));
  const pending = calls.find((c) => c.status !== 'done');
  const state = pending ? callState(pending) : null;
  const proposals = tickets.length > 0 && tickets.every((t) => t.proposal);
  const n = tickets.length;
  const heading = n === 0
    ? pending?.status === 'running' ? 'Creating a ticket…' : "Didn't create a ticket"
    : `${proposals ? 'Proposed' : 'Created'} ${n === 1 ? 'a ticket' : `${n} tickets`}`;
  return (
    <Card icon={<Plus />} label={heading} heading={heading} aside={state?.aside}>
      {tickets.map((t, i) => <TicketRow key={t.key ?? i} t={t} />)}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** A changed field's value as text: a stage or priority by its label, a list joined. */
function valueText(field: string, v: unknown): string {
  if (v === null || v === undefined || v === '') return 'none';
  if (Array.isArray(v)) return v.length ? v.join(', ') : 'none';
  if (field === 'stage') return stageLabel(String(v));
  if (field === 'priority') return priorityLabel(String(v));
  return String(v);
}

const FIELD_LABEL: Record<string, string> = {
  title: 'Title', stage: 'Stage', priority: 'Priority', labels: 'Labels', areas: 'Areas', parent: 'Part of', description: 'Description',
};

export function ChangeRow({ c }: { c: FieldChange }) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 py-2">
      <span className="w-20 shrink-0 text-xs text-muted-foreground">{FIELD_LABEL[c.field] ?? c.field}</span>
      {c.field === 'description' ? (
        <span>Rewritten</span>
      ) : (
        <span className="flex min-w-0 flex-wrap items-center gap-1.5">
          {c.from !== undefined && (
            <>
              <span className="text-muted-foreground line-through decoration-border">{valueText(c.field, c.from)}</span>
              <ArrowRight aria-hidden className="size-3 text-muted-foreground" /><span className="sr-only">to</span>
            </>
          )}
          <span className="font-medium">{valueText(c.field, c.to)}</span>
        </span>
      )}
    </div>
  );
}

/** `edit_ticket`: from → to for each changed field. */
function EditTicketCard({ call }: { call: PdtCall }) {
  const e = ticketEdit(call.input, call.output);
  const state = callState(call);
  const verb = call.status === 'done' ? 'Changed' : call.status === 'running' ? 'Changing' : 'Not changed:';
  return (
    <Card icon={<Pencil />} label={`${verb} ${e.key}`} heading={<>{verb} <Key value={e.key} /></>} aside={state?.aside}>
      {e.changes.map((c) => <ChangeRow key={c.field} c={c} />)}
      {e.files > 0 && <div className="py-2 text-xs text-muted-foreground">Attached {e.files === 1 ? 'a file' : `${e.files} files`}</div>}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `link_tickets` / `unlink_tickets`: "PDT-2 blocked by PDT-1". */
function LinkCard({ call }: { call: PdtCall }) {
  const l = ticketLink(call.input);
  const unlink = pdtTool(call.tool) === 'unlink_tickets';
  const state = callState(call);
  const heading = unlink ? 'Unlinked' : 'Linked';
  return (
    <Card icon={<GitFork />} label={heading} heading={heading} aside={state?.aside}>
      <div className={cn('flex flex-wrap items-center gap-2 py-2', unlink && 'text-muted-foreground')}>
        <Key value={l.ticket} />
        <span>{unlink ? `no longer ${LINK_TEXT[l.kind]}` : LINK_TEXT[l.kind]}</span>
        <Key value={l.other} />
      </div>
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `send_to_session`: the quoted message, who gets it, and that it is queued (delivered when that session is free). */
function MessageCard({ call }: { call: PdtCall }) {
  const m = sentMessage(call.input, call.output);
  const state = callState(call);
  const to = m.to ?? 'a session';
  return (
    <Card
      icon={<Mail />}
      label={`Message to ${to}`}
      heading={<>Message to <span className="font-medium text-foreground">{to}</span>{m.ticket && !m.to?.includes(m.ticket) && <Key value={m.ticket} />}</>}
      aside={state?.aside ?? <span title="Delivered when the session next takes input">Queued</span>}
    >
      <blockquote className="my-2 border-l-2 border-border pl-3 text-muted-foreground whitespace-pre-wrap">{m.text}</blockquote>
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `add_ticket_comment`: the comment, previewed. */
function CommentCard({ call }: { call: PdtCall }) {
  const c = ticketComment(call.input, call.output);
  const state = callState(call);
  const verb = call.status === 'done' ? 'Commented on' : call.status === 'running' ? 'Commenting on' : 'Not commented on';
  return (
    <Card icon={<MessageSquare />} label={`${verb} ${c.key ?? 'the ticket'}`} heading={<>{verb} <Key value={c.key} /></>} aside={state?.aside}>
      <div className="max-h-32 overflow-hidden py-2">
        <Markdown>{c.body}</Markdown>
      </div>
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `workflow_run`: the run, with its status (followed while it runs). */
function WorkflowRunCard({ call }: { call: PdtCall }) {
  const w = workflowRun(call.input, call.output);
  const state = callState(call);
  const run = useQuery({
    queryKey: workflowKeys.run(w.runId ?? ''),
    queryFn: () => api.getWorkflowRun(w.runId!),
    enabled: !!w.runId,
    refetchInterval: (q) => (q.state.data && ['running', 'waiting'].includes(q.state.data.status) ? POLL.steady : false),
  });
  const r = run.data;
  const verb = call.status === 'done' ? 'Started' : call.status === 'running' ? 'Starting' : "Didn't start";
  return (
    <Card
      icon={<Play />}
      label={`${verb} ${w.workflow}`}
      heading={<>{verb} <span className="font-medium text-foreground">{r?.workflowName ?? w.workflow}</span></>}
      aside={state?.aside ?? (r ? <Badge tone={RUN_STATUS[r.status].tone}>{RUN_STATUS[r.status].text}</Badge> : null)}
    >
      {r && (
        <div className="flex items-center gap-2 py-2 text-xs">
          {r.error && <span className="min-w-0 flex-1 truncate text-failed" title={r.error}>{r.error}</span>}
          <span className="flex-1" />
          <a href={`/workflows/${encodeURIComponent(r.workflowId)}/runs/${encodeURIComponent(r.id)}`} className="relative touch-hit text-link underline-offset-2 hover:underline">
            Open the run
          </a>
        </div>
      )}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `watch`: what is watched and what it tells about, with Stop watching while it lasts. */
function WatchCard({ call }: { call: PdtCall }) {
  const qc = useQueryClient();
  const w = watchOf(call.input, call.output);
  const session = (resultOf(call.output)?.watching as { sessionId?: string }[] | undefined)?.[0]?.sessionId ?? null;
  const state = callState(call);
  const list = useQuery({
    queryKey: ['watches', { session }],
    queryFn: () => api.watches.list({ session: session! }),
    enabled: !!w.id && !!session,
  });
  const stop = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.watches.remove(w.id!),
    onSettled: () => qc.invalidateQueries({ queryKey: ['watches'] }),
  });
  const watching = !!w.id && call.status === 'done' && !stop.isSuccess && (list.data ? list.data.some((x) => x.id === w.id) : true);
  const what = w.target === 'ticket' ? <Key value={w.label} /> : <span className="font-medium text-foreground">{w.label}</span>;
  return (
    <Card
      icon={<Eye />}
      label={`${watching ? 'Watching' : 'Watched'} ${w.label}`}
      heading={
        <>
          {watching ? 'Watching' : call.status === 'done' ? 'No longer watching' : 'Watch'} {what}
          {watching && <span>· you'll be told when {watchEventsText(w.events)}</span>}
        </>
      }
      aside={state?.aside ?? (watching && (
        <Button variant="ghost" size="sm" pending={stop.isPending} disabled={stop.isPending} onClick={() => stop.mutate()}>
          Stop watching
        </Button>
      ))}
    >
      {stop.isError && <Failure text={actionError(stop.error)} />}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `unwatch`: one line. */
function UnwatchCard({ call }: { call: PdtCall }) {
  const state = callState(call);
  return <Card icon={<Eye />} label="Stopped watching" heading="Stopped watching" aside={state?.aside}>{state?.error && <Failure text={state.error} />}</Card>;
}

/** "Stopping", "Stopped", "Didn't stop": a card's verb for where its call is. */
function verbFor(call: PdtCall, running: string, done: string, not: string): string {
  return call.status === 'done' ? done : call.status === 'running' ? running : not;
}

/** `stop_ticket`: the ticket, where it went, and what stopped. */
function StopTicketCard({ call }: { call: PdtCall }) {
  const t = stoppedTicket(call.input, call.output);
  const state = callState(call);
  const verb = verbFor(call, 'Stopping work on', 'Stopped work on', "Didn't stop work on");
  const stopped = [
    t.runsCancelled > 0 && `cancelled ${plural(t.runsCancelled, 'workflow run')}`,
    t.sessionsStopped > 0 && `stopped ${plural(t.sessionsStopped, 'session')}`,
    t.worktreeRemoved && 'removed its worktree',
  ].filter(Boolean) as string[];
  const done = call.status === 'done';
  return (
    <Card
      icon={<CircleStop />}
      label={`${verb} ${t.key}`}
      heading={<>{verb} <Key value={t.key} /></>}
      aside={state?.aside ?? (t.stage && <Badge>{stageLabel(t.stage)}</Badge>)}
    >
      {done && (stopped.length > 0 || t.branchNote) && (
        <div className="flex flex-col gap-0.5 py-2 text-xs text-muted-foreground">
          {stopped.length > 0 && <span>{stopped.join(', ').replace(/^./, (c) => c.toUpperCase())}</span>}
          {t.branchNote && <span>{t.branchNote}</span>}
        </div>
      )}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** An issue's key (`owner/repo#12`) linking to it on its tracker. */
function IssueKey({ issue, url }: { issue: string | null; url: string | null }) {
  if (!issue && !url) return null;
  const text = issue ?? url!;
  return url ? (
    <a href={url} target="_blank" rel="noreferrer" className="relative touch-hit shrink-0 font-mono text-meta text-link underline-offset-2 hover:underline">{text}</a>
  ) : (
    <span className="shrink-0 font-mono text-meta">{text}</span>
  );
}

/** `issue_create` / `report_bug`: the issue filed (its title, key and link), and the ticket that links it. */
function IssueCard({ call }: { call: PdtCall }) {
  const bug = pdtTool(call.tool) === 'report_bug';
  const i = filedIssue(call.input, call.output);
  const state = callState(call);
  const heading = bug
    ? verbFor(call, 'Reporting a bug in PDT…', 'Reported a bug in PDT', "Didn't report the bug")
    : verbFor(call, 'Opening an issue…', 'Opened an issue', "Didn't open an issue");
  // A bug report that became a local ticket (no issue) names the ticket instead.
  const asTicket = bug && call.status === 'done' && !i.url && i.ticket;
  return (
    <Card icon={bug ? <Bug /> : <CircleDot />} label={heading} heading={heading} aside={state?.aside}>
      {(i.title || i.issue || i.url || asTicket) && (
        <div className="flex flex-wrap items-center gap-2 py-2">
          <IssueKey issue={i.issue} url={i.url} />
          {asTicket && <Key value={i.ticket} />}
          {i.title && <span className="min-w-0 flex-1 font-medium">{i.title}</span>}
          {!bug && i.ticket && <span className="flex items-center gap-1.5 text-xs text-muted-foreground">from <Key value={i.ticket} /></span>}
        </div>
      )}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `link_issue` / `unlink_issue`: the ticket and the issue (or plain link) linked to it or removed. */
function IssueLinkCard({ call }: { call: PdtCall }) {
  const unlink = pdtTool(call.tool) === 'unlink_issue';
  const l = issueLink(call.input, call.output);
  const state = callState(call);
  const verb = unlink
    ? verbFor(call, 'Unlinking an issue from', 'Unlinked an issue from', "Didn't unlink an issue from")
    : verbFor(call, 'Linking an issue to', 'Linked an issue to', "Didn't link an issue to");
  return (
    <Card icon={<Link2 />} label={`${verb} ${l.ticket}`} heading={<>{verb} <Key value={l.ticket} /></>} aside={state?.aside}>
      <div className={cn('flex flex-wrap items-center gap-2 py-2', unlink && 'text-muted-foreground')}>
        <IssueKey issue={l.key} url={l.url} />
        {l.title && <span className={cn('min-w-0 flex-1', !unlink && 'font-medium')}>{l.title}</span>}
      </div>
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** Opens a document beside this conversation (as `document_open_beside` does). */
function OpenBeside({ documentId }: { documentId: string }) {
  const qc = useQueryClient();
  const sessionId = useContext(MarkdownRefsContext)?.sessionId ?? null;
  const [open, pending] = useAction(async () => {
    qc.setQueryData(besideKey(sessionId!), await api.openBeside(sessionId!, documentId));
  });
  if (!sessionId) return null;
  return (
    <Button variant="ghost" size="sm" pending={pending} disabled={pending} onClick={() => void open()}>
      <PanelRight aria-hidden /> Open beside
    </Button>
  );
}

const DOC_VERBS: Record<string, [string, string, string]> = {
  document_create: ['Creating', 'Created', "Didn't create"],
  doc_edit: ['Editing', 'Edited', "Didn't edit"],
  sheet_edit: ['Editing', 'Edited', "Didn't edit"],
};

/** `document_create`, `doc_edit`, `sheet_edit`, `document_link`: the document, its kind, what changed, its links. */
function DocumentCard({ call }: { call: PdtCall }) {
  const name = pdtTool(call.tool) ?? '';
  const d = documentOf(call.tool, call.input, call.output);
  const state = callState(call);
  const unlink = name === 'document_link' && (call.input as { unlink?: unknown } | null)?.unlink === true;
  const [running, done, not] = DOC_VERBS[name] ?? (unlink ? ['Unlinking', 'Unlinked', "Didn't unlink"] : ['Linking', 'Linked', "Didn't link"]);
  const verb = verbFor(call, running, done, not);
  const diff = name === 'doc_edit' && call.status === 'done' ? diffOf(call.output) : null;
  const stats = diff ? diffStats(diff) : null;
  const title = d.id && call.status === 'done'
    ? <a href={routePath({ view: 'documents', doc: d.id })} className="relative touch-hit font-medium text-foreground underline-offset-2 hover:underline">{d.title}</a>
    : <span className="font-medium text-foreground">{d.title}</span>;
  const links = d.tickets.length + d.projects.length > 0;
  return (
    <Card
      icon={d.kind === 'sheet' ? <Sheet /> : <FileText />}
      label={`${verb} ${d.title}`}
      heading={<>{verb} {title}{d.kind && <Badge>{d.kind === 'sheet' ? 'Sheet' : 'Doc'}</Badge>}</>}
      aside={state?.aside ?? (d.id && call.status === 'done' && (d.openedBeside ? <span>Open beside</span> : <OpenBeside documentId={d.id} />))}
    >
      {d.change && (
        <div className="flex items-baseline gap-2 py-2">
          <span className="min-w-0 flex-1">{d.change}</span>
          {stats && <DiffStat added={stats.added} removed={stats.removed} />}
        </div>
      )}
      {links && call.status === 'done' && (
        <div className="flex flex-wrap items-center gap-1.5 py-2 text-xs text-muted-foreground">
          Linked to
          {d.tickets.map((t) => <Key key={t} value={t} />)}
          {d.projects.map((p) => <Badge key={p}>{p}</Badge>)}
        </div>
      )}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** A `doc_edit` result's diff, if it has one. */
function diffOf(output?: string): string | null {
  const v = resultOf(output)?.diff;
  return typeof v === 'string' && v ? v : null;
}

/** `save_note` / `delete_note`: the note's title (its body folded under it), its project and whether it is pinned. */
function NoteCard({ call }: { call: PdtCall }) {
  const del = pdtTool(call.tool) === 'delete_note';
  const note = noteOf(call.input, call.output);
  const state = callState(call);
  const heading = del
    ? verbFor(call, 'Deleting a note…', 'Deleted a note', "Didn't delete the note")
    : note.replaced
      ? verbFor(call, 'Updating a note…', 'Updated a note', "Didn't update the note")
      : verbFor(call, 'Saving a note…', 'Saved a note', "Didn't save the note");
  const known = !del || call.status === 'done';
  const badges = known && (
    <span className="flex shrink-0 items-center gap-1">
      <Badge>{note.project ?? 'Host-wide'}</Badge>
      {note.pinned && <Badge><Pin aria-hidden /> Pinned</Badge>}
    </span>
  );
  const title = note.title ?? (del ? `Note ${note.id ?? ''}`.trim() : 'Untitled');
  return (
    <Card icon={del ? <Trash2 /> : <StickyNote />} label={heading} heading={heading} aside={state?.aside}>
      <div className={cn('flex items-start gap-2 py-2', del && 'text-muted-foreground')}>
        {note.body ? (
          <Disclosure title={<span className="font-medium">{title}</span>} className="min-w-0 flex-1" panelClassName="pt-1 text-sm">
            <Markdown>{note.body}</Markdown>
          </Disclosure>
        ) : (
          <span className={cn('min-w-0 flex-1', !del && 'font-medium', del && call.status === 'done' && 'line-through decoration-border')}>{title}</span>
        )}
        {badges}
      </div>
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** The card for a PDT tool call (one of `isCardTool`'s). */
export function PdtToolCard({ call }: { call: PdtCall }) {
  switch (pdtTool(call.tool)) {
    case 'create_ticket':
      return <CreateTicketsCard calls={[call]} />;
    case 'edit_ticket':
      return <EditTicketCard call={call} />;
    case 'link_tickets':
    case 'unlink_tickets':
      return <LinkCard call={call} />;
    case 'send_to_session':
      return <MessageCard call={call} />;
    case 'add_ticket_comment':
      return <CommentCard call={call} />;
    case 'workflow_run':
      return <WorkflowRunCard call={call} />;
    case 'watch':
      return <WatchCard call={call} />;
    case 'unwatch':
      return <UnwatchCard call={call} />;
    case 'stop_ticket':
      return <StopTicketCard call={call} />;
    case 'issue_create':
    case 'report_bug':
      return <IssueCard call={call} />;
    case 'link_issue':
    case 'unlink_issue':
      return <IssueLinkCard call={call} />;
    case 'document_create':
    case 'doc_edit':
    case 'sheet_edit':
    case 'document_link':
      return <DocumentCard call={call} />;
    case 'save_note':
    case 'delete_note':
      return <NoteCard call={call} />;
    default:
      return null;
  }
}
