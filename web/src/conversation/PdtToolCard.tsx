// PDT's own tool calls in a conversation as cards (PDT-382): what the agent created, changed, linked, sent, ran or
// watched, instead of a folded "Used N tools" line.
import type { ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowRight, Eye, GitFork, Mail, MessageSquare, Pencil, Play, Plus } from 'lucide-react';
import { Badge, Button, Spinner, type BadgeTone } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { Priority } from '../api/gen/Priority';
import type { TicketStage } from '../api/gen/TicketStage';
import type { WorkflowRunStatus } from '../api/gen/WorkflowRunStatus';
import { Markdown } from '../common/Markdown';
import { PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { TicketPill } from '../common/TicketPill';
import { actionError } from '../common/useAction';
import { RUN_STATUS } from '../workflows/runs';
import {
  createdTicket,
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
              <ArrowRight aria-label="to" className="size-3 text-muted-foreground" />
            </>
          )}
          <span className="font-medium">{valueText(c.field, c.to)}</span>
        </span>
      )}
    </div>
  );
}

/** `edit_ticket`: from → to for each changed field. */
export function EditTicketCard({ call }: { call: PdtCall }) {
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
export function LinkCard({ call }: { call: PdtCall }) {
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
export function MessageCard({ call }: { call: PdtCall }) {
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
export function CommentCard({ call }: { call: PdtCall }) {
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
export function WorkflowRunCard({ call }: { call: PdtCall }) {
  const w = workflowRun(call.input, call.output);
  const state = callState(call);
  const run = useQuery({
    queryKey: ['workflow-run', w.runId],
    queryFn: () => api.getWorkflowRun(w.runId!),
    enabled: !!w.runId,
    refetchInterval: (q) => (q.state.data && ['running', 'waiting'].includes(q.state.data.status) ? 5_000 : false),
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
          <a href={`/workflows/${encodeURIComponent(r.workflowId)}/runs/${encodeURIComponent(r.id)}`} className="text-link underline-offset-2 hover:underline">
            Open the run
          </a>
        </div>
      )}
      {state?.error && <Failure text={state.error} />}
    </Card>
  );
}

/** `watch`: what is watched and what it tells about, with Stop watching while it lasts. */
export function WatchCard({ call }: { call: PdtCall }) {
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
          {watching && <span>· you'll be told when it {watchEventsText(w.events)}</span>}
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
export function UnwatchCard({ call }: { call: PdtCall }) {
  const state = callState(call);
  return <Card icon={<Eye />} label="Stopped watching" heading="Stopped watching" aside={state?.aside}>{state?.error && <Failure text={state.error} />}</Card>;
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
    default:
      return null;
  }
}
