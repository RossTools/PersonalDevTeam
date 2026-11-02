// The pieces of a ticket's Overview, shared by the ticket tab (TicketView) and the backlog's peek sheet.
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowRight, CircleCheck, Clock, GitPullRequest, Link2, ListTree, MessageSquare, Pencil, ShieldAlert, TriangleAlert } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import {
  Alert, Badge, Button, Chip, Dialog, IconButton, MenuButton, SegmentedControl, Select, Textarea, Timeline, toast, useConfirm,
  type TimelineItem,
} from '@ui';
import { api } from '../api/client';
import type { AgentBy } from '../api/gen/AgentBy';
import type { LinkedTicket } from '../api/gen/LinkedTicket';
import type { Priority } from '../api/gen/Priority';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketEvent } from '../api/gen/TicketEvent';
import type { TicketOverlap } from '../api/gen/TicketOverlap';
import type { TicketStage } from '../api/gen/TicketStage';
import { Composer } from '../common/Composer';
import { insertAtCaret } from '../common/insertText';
import { InlineRename } from '../common/InlineRename';
import { Markdown } from '../common/Markdown';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { useWatches, Watchers } from '../common/Watch';
import { actionError, useAction } from '../common/useAction';
import { moveTicket, patchTicket, useOpenSession, useOpenTicket } from './actions';
import { ParentField } from './PartsSection';
import { eventTitle, LINK_ROLE_LABEL, linkCall, linkRole, STAGES, type LinkRole } from './model';
import { invalidateTickets, pagedPoll, POLL, ticketKeys, useLabels, useProjectAreas } from './queries';
import { AreaChips, AreasEditor } from './Areas';
import { ATTACH_ACCEPT, AttachButton, attachmentMarkdown, UploadList, useAttachmentUploads } from './attachments';
import { LabelsEditor } from './LabelsEditor';
import { TicketPicker } from './TicketPicker';

const sectionHeading = 'text-2xs font-medium tracking-wider text-muted-foreground uppercase';

/** True inside a folding section that already names the block (the phone's ticket screen): its own heading hides. */
export const FoldedSectionContext = createContext(false);

/** A titled block of the Overview ("Details", "Links"). */
export function OverviewSection({ title, actions, children, className }: { title: string; actions?: ReactNode; children: ReactNode; className?: string }) {
  const folded = useContext(FoldedSectionContext);
  return (
    <section aria-label={title} className={cn('flex flex-col gap-2', className)}>
      {folded && !actions ? <h3 className="sr-only">{title}</h3> : (
        <div className="flex min-h-6 items-center gap-2">
          <h3 className={cn(sectionHeading, 'flex-1', folded && 'sr-only')}>{title}</h3>
          {folded && <span className="flex-1" />}
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Title

/**
 * The title with a pencil: edits in place (Enter or leaving the field saves, Escape cancels). The new title shows
 * while it saves; if saving fails, the editor opens again with what was typed.
 */
export function TicketTitle({ ticket, className }: { ticket: Pick<Ticket, 'id' | 'title'>; className?: string }) {
  const qc = useQueryClient();
  // The text being edited (the title, or what was typed before a failed save), or null when not editing.
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const save = async (title: string) => {
    setDraft(null);
    setSaving(title);
    try {
      await patchTicket(qc, ticket.id, { title });
    } catch (e) {
      toast.error(actionError(e));
      setDraft(title);
    } finally {
      setSaving(null);
    }
  };
  if (draft !== null) {
    return (
      <InlineRename
        label="Title"
        initial={ticket.title}
        draft={draft}
        maxLength={200}
        size="sm"
        className={cn('w-full min-w-48', className)}
        onCommit={(t) => void save(t)}
        onCancel={() => setDraft(null)}
      />
    );
  }
  const shown = saving ?? ticket.title;
  return (
    <span className={cn('group/title flex min-w-0 items-center gap-1', className)}>
      <span className="min-w-0 truncate" title={shown} onDoubleClick={() => setDraft(ticket.title)}>{shown}</span>
      <IconButton label="Edit title" size="sm" noTooltip disabled={saving !== null} className="size-5 shrink-0" onClick={() => setDraft(ticket.title)}>
        <Pencil aria-hidden />
      </IconButton>
    </span>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Description

/**
 * The description as Markdown; Edit switches to a text box with a Preview toggle, saved with Save. The edit is
 * measured against the description it started from, not the live one the poll brings, so a change made meanwhile (an
 * agent's rewrite) is shown as a warning instead of being silently overwritten. `onDirtyChange` tells the parent
 * whether there is an unsaved edit (the peek asks before closing over one).
 */
export function TicketDescription({ ticket, onDirtyChange }: {
  ticket: Pick<Ticket, 'id' | 'description'>;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const qc = useQueryClient();
  // What is typed, and the description it was started from.
  const [edit, setEdit] = useState<{ draft: string; base: string } | null>(null);
  const [mode, setMode] = useState<'write' | 'preview'>('write');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const editing = edit !== null;
  const draft = edit?.draft ?? '';
  const dirty = editing && edit.draft !== edit.base;
  const changedElsewhere = editing && edit.base !== ticket.description;
  const setDraft = (d: string) => setEdit((e) => (e ? { ...e, draft: d } : e));
  const box = useRef<HTMLTextAreaElement>(null);
  const { uploads, add, dismiss } = useAttachmentUploads(ticket.id);
  const attach = async (files: File[]) => {
    const snippets = (await add(files)).map(attachmentMarkdown);
    if (snippets.length) setEdit((e) => (e ? { ...e, draft: insertAtCaret(e.draft, box.current, snippets.join('\n')) } : e));
  };
  const begin = () => {
    setEdit({ draft: ticket.description, base: ticket.description });
    setMode('write');
  };
  const close = () => {
    setEdit(null);
    setError(null);
  };
  // Nothing typed yet: follow the description as it changes.
  const live = ticket.description;
  useEffect(() => {
    if (!dirty && changedElsewhere) setEdit({ draft: live, base: live });
  }, [dirty, changedElsewhere, live]);
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const save = async () => {
    if (!dirty) return;
    setPending(true);
    setError(null);
    try {
      await patchTicket(qc, ticket.id, { description: draft });
      setEdit(null);
    } catch (e) {
      setError(actionError(e));
    } finally {
      setPending(false);
    }
  };
  return (
    <OverviewSection
      title="Description"
      actions={!editing && (
        <Button variant="ghost" size="sm" compact onClick={begin}>
          <Pencil aria-hidden /> Edit
        </Button>
      )}
    >
      {editing ? (
        <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); void save(); }}>
          <SegmentedControl
            aria-label="Description editor"
            size="sm"
            className="self-start"
            options={[{ value: 'write', label: 'Write' }, { value: 'preview', label: 'Preview' }]}
            value={mode}
            onChange={setMode}
          />
          {mode === 'write' ? (
            <Textarea
              ref={box}
              aria-label="Description"
              autoFocus
              rows={10}
              value={draft}
              placeholder="Markdown: what and why, acceptance criteria… Paste or drop screenshots and logs."
              onChange={(e) => setDraft(e.target.value)}
              onPaste={(e) => {
                const files = [...e.clipboardData.files];
                if (files.length) { e.preventDefault(); void attach(files); }
              }}
              onDragOver={(e) => { if (e.dataTransfer.types.includes('Files')) e.preventDefault(); }}
              onDrop={(e) => {
                const files = [...e.dataTransfer.files];
                if (files.length) { e.preventDefault(); void attach(files); }
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void save(); }
                if (e.key === 'Escape' && !dirty) { e.preventDefault(); close(); }
              }}
              className="min-h-40 resize-y"
            />
          ) : (
            <div className="min-h-40 rounded-md border border-border p-2">
              {draft.trim() ? <Markdown breaks>{draft}</Markdown> : <p className="text-xs text-muted-foreground">Nothing to preview.</p>}
            </div>
          )}
          {changedElsewhere && (
            <Alert
              tone="warning"
              action={<Button size="sm" variant="secondary" onClick={begin}>Load the new version</Button>}
            >
              The description changed while you were editing it. Save replaces that change with yours; loading the new
              version drops your edit.
            </Alert>
          )}
          {error && <Alert tone="error">{error}</Alert>}
          <UploadList uploads={uploads} onDismiss={dismiss} />
          <div className="flex items-center justify-end gap-2">
            <span className="mr-auto text-meta text-muted-foreground pointer-coarse:hidden">Markdown. ⌘/Ctrl+Enter saves.</span>
            <AttachButton onFiles={(f) => void attach(f)} />
            <Button variant="ghost" onClick={close}>Cancel</Button>
            <Button type="submit" variant="primary" pending={pending} disabled={!dirty}>Save</Button>
          </div>
        </form>
      ) : ticket.description.trim() ? (
        <Markdown breaks>{ticket.description}</Markdown>
      ) : (
        <p className="text-xs text-muted-foreground">No description.</p>
      )}
    </OverviewSection>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Details

const stageOptions = STAGES.map((s) => ({ value: s, label: STAGE_LABEL[s] }));
const priorityOptions = PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }));

function DetailRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[5rem_minmax(0,1fr)] items-center gap-2 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** Stage, priority, labels and areas; each change saves at once. Shown in the ticket tab and the peek. */
export function TicketDetails({ ticket }: { ticket: Ticket }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const labels = useLabels(ticket.projectId);
  const [save, saving] = useAction((patch: Parameters<typeof patchTicket>[2]) => patchTicket(qc, ticket.id, patch));
  // A parent with open parts can't close: moveTicket offers to close them first.
  const [move, moving] = useAction((stage: TicketStage) => moveTicket(qc, ticket, stage, confirm));
  return (
    <OverviewSection title="Details">
      <DetailRow label="Stage">
        <Select aria-label="Stage" size="sm" options={stageOptions} value={ticket.stage} disabled={saving || moving}
          onChange={(stage: TicketStage) => void move(stage)} />
      </DetailRow>
      {ticket.partList.length === 0 && (
        <DetailRow label="Parent">
          <ParentField ticket={ticket} />
        </DetailRow>
      )}
      <DetailRow label="Priority">
        <Select aria-label="Priority" size="sm" options={priorityOptions} value={ticket.priority} disabled={saving}
          onChange={(priority: Priority) => void save({ priority })} />
      </DetailRow>
      <DetailRow label="Labels">
        <LabelsEditor labels={ticket.labels} suggestions={labels.data ?? []} onChange={(l) => void save({ labels: l })} />
      </DetailRow>
      {/* The parts of the project the ticket expects to touch: overlap is checked on the ones .pdt/project.toml declares. */}
      <DetailRow label="Areas">
        <div role="group" aria-label="Areas">
          <AreasEditor projectId={ticket.projectId} areas={ticket.areas} onChange={(areas) => void save({ areas })} />
        </div>
      </DetailRow>
    </OverviewSection>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Overlaps

/** Shared files listed before "and N more". */
export const OVERLAP_FILES_SHOWN = 5;

/** The files two tickets' attempts both changed: the first few, then "and N more" (of `total`). */
export function SharedFiles({ files, total, className }: { files: string[]; total: number; className?: string }) {
  if (files.length === 0) return null;
  const shown = files.slice(0, OVERLAP_FILES_SHOWN);
  const more = Math.max(total, files.length) - shown.length;
  return (
    <ul aria-label="Shared files" className={cn('flex min-w-0 flex-col gap-0.5 font-mono text-meta', className)}>
      {shown.map((f) => <li key={f} className="min-w-0 truncate" title={f}>{f}</li>)}
      {more > 0 && <li className="font-sans text-muted-foreground">and {more} more</li>}
    </ul>
  );
}

function OverlapRow({ ticket, overlap: o }: { ticket: Ticket; overlap: TicketOverlap }) {
  const open = useOpenTicket();
  const areas = useProjectAreas(ticket.projectId);
  const config = areas.data && !areas.data.error ? areas.data.areas : undefined;
  return (
    <li aria-label={`${o.key} ${o.title}`} className="flex min-w-0 flex-col gap-1 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        <TicketKey ticketKey={o.key} title={o.title} onClick={() => open({ id: o.ticketId, projectId: ticket.projectId })} />
        <span className="min-w-0 flex-1 truncate">{o.title}</span>
        <Badge tone={o.level === 'conflict' ? 'failed' : 'waiting'}>{o.level === 'conflict' ? 'Conflict' : 'Overlap'}</Badge>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-1 pl-1 text-meta text-muted-foreground">
        <span>{STAGE_LABEL[o.stage]}</span>
        {o.areas.length > 0 && <><span aria-hidden>·</span><AreaChips areas={o.areas} config={config} /></>}
      </div>
      <SharedFiles files={o.files} total={o.filesTotal} className="pl-1" />
    </li>
  );
}

/** The project's active tickets this one shares areas or changed files with, conflicts first (only when there are any). */
export function TicketOverlaps({ ticket }: { ticket: Ticket }) {
  if (ticket.overlaps.length === 0) return null;
  return (
    <OverviewSection title="Overlaps">
      <ul aria-label="Overlaps" className="flex flex-col gap-3">
        {ticket.overlaps.map((o) => <OverlapRow key={o.ticketId} ticket={ticket} overlap={o} />)}
      </ul>
    </OverviewSection>
  );
}

/** Who watches the ticket (PDT-319), with remove; nothing when nobody does. */
export function TicketWatchers({ ticketId }: { ticketId: string }) {
  const watches = useWatches({ ticket: ticketId }).data ?? [];
  if (watches.length === 0) return null;
  return (
    <OverviewSection title="Watching">
      <Watchers of={{ ticket: ticketId }} />
    </OverviewSection>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Links

const ROLES: LinkRole[] = ['blockedBy', 'blocks', 'related'];

function LinkRow({ ticket, link }: { ticket: Ticket; link: LinkedTicket }) {
  const qc = useQueryClient();
  const open = useOpenTicket();
  const [remove] = useAction(async () => {
    const c = linkCall(ticket.id, link.id, linkRole(link));
    await api.unlinkTicket(c.from, { to: c.to, kind: c.kind });
    invalidateTickets(qc, [ticket.id, link.id]);
  });
  const openIt = () => open({ id: link.id, projectId: ticket.projectId });
  return (
    <li className="flex min-w-0 items-center gap-2 text-xs">
      <TicketKey ticketKey={link.key} title={link.title} onClick={openIt} />
      <span className={cn('min-w-0 flex-1 truncate', (link.stage === 'done' || link.stage === 'wont_do') && 'text-muted-foreground line-through')}>
        {link.title}
      </span>
      <Badge>{STAGE_LABEL[link.stage]}</Badge>
      <MenuButton
        label={`Link to ${link.key}`}
        size="sm"
        entries={() => [
          { label: 'Open', onSelect: openIt },
          { label: 'Remove link', danger: true, onSelect: () => void remove() },
        ]}
      />
    </li>
  );
}

/** Blocked by / Blocks / Related, each ticket with its key, title and stage; "Link ticket…" adds one. */
export function TicketLinks({ ticket }: { ticket: Ticket }) {
  const [linking, setLinking] = useState(false);
  const byRole = (r: LinkRole) => ticket.links.filter((l) => linkRole(l) === r);
  return (
    <OverviewSection
      title="Links"
      actions={
        <Button variant="ghost" size="sm" compact onClick={() => setLinking(true)}>
          <Link2 aria-hidden /> Link ticket…
        </Button>
      }
    >
      {ticket.links.length === 0 && <p className="text-xs text-muted-foreground">No links.</p>}
      {ROLES.map((r) => {
        const list = byRole(r);
        if (list.length === 0) return null;
        return (
          <div key={r} className="flex flex-col gap-1">
            <h4 className="text-meta text-muted-foreground">{LINK_ROLE_LABEL[r]}</h4>
            <ul aria-label={LINK_ROLE_LABEL[r]} className="flex flex-col gap-1">
              {list.map((l) => <LinkRow key={`${l.kind}:${l.direction}:${l.id}`} ticket={ticket} link={l} />)}
            </ul>
          </div>
        );
      })}
      {linking && <LinkTicketDialog ticket={ticket} onClose={() => setLinking(false)} />}
    </OverviewSection>
  );
}

/** Pick the kind (blocked by, blocks, related), then the ticket. A link that would close a cycle shows why here. */
export function LinkTicketDialog({ ticket, onClose }: { ticket: Ticket; onClose: () => void }) {
  const qc = useQueryClient();
  const [role, setRole] = useState<LinkRole>('blockedBy');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const link = async (otherId: string) => {
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      const c = linkCall(ticket.id, otherId, role);
      await api.linkTicket(c.from, { to: c.to, kind: c.kind });
      invalidateTickets(qc, [ticket.id, otherId]);
      onClose();
    } catch (e) {
      setError(actionError(e));
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title={`Link ${ticket.key} to…`} size="md">
      <div className="flex flex-col gap-2">
        <SegmentedControl
          aria-label="Link kind"
          size="sm"
          className="self-start"
          options={ROLES.map((r) => ({ value: r, label: LINK_ROLE_LABEL[r] }))}
          value={role}
          onChange={setRole}
        />
        <TicketPicker
          projectId={ticket.projectId}
          exclude={[ticket.id]}
          label="Ticket to link"
          onPick={(t) => void link(t.id)}
          className="max-h-[50dvh]"
        />
        {error && <Alert tone="error">{error}</Alert>}
      </div>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Timeline

const EVENT_PAGE = 50;

function iconFor(e: TicketEvent) {
  // An agent's comment (posted through the API) keeps the agent's icon, so it doesn't pass for the user's.
  if (e.kind === 'comment' && e.actor === 'agent') return undefined;
  if (e.kind === 'comment') return <MessageSquare aria-hidden />;
  if (e.kind === 'moved') return <ArrowRight aria-hidden />;
  if (e.kind === 'linked' || e.kind === 'unlinked') return <Link2 aria-hidden />;
  if (e.kind === 'waiting') return <Clock aria-hidden />;
  if (e.kind === 'overlap') return (e.data as { cleared?: boolean } | null)?.cleared ? <CircleCheck aria-hidden /> : <TriangleAlert aria-hidden />;
  if (e.kind === 'pr') return <GitPullRequest aria-hidden />;
  if (e.kind === 'parted') return <ListTree aria-hidden />;
  if (e.kind === 'outside_input') return <ShieldAlert aria-hidden />;
  return undefined;
}

/** What an agent step's outcome event carries (spec 7b §4). */
interface OutcomeData {
  summary?: string;
  persona?: string;
  workflowName?: string;
}

/** Who the timeline names: an agent's events read as by its persona (or "Agent"), never as by you. */
function actorOf(e: TicketEvent): string {
  if (e.actor !== 'agent') return e.actor;
  if (e.agent?.persona) return e.agent.persona;
  const persona = (e.data as { persona?: unknown } | null)?.persona;
  return typeof persona === 'string' && persona ? persona : 'Agent';
}

/**
 * An agent's persona and the ticket its session works on ("Coder · PDT-34"; "Coder · from PDT-34" for a ticket it
 * made), opening that session (PDT-163).
 */
function AgentLabel({ agent, name, made = false }: { agent: AgentBy; name: string; made?: boolean }) {
  const openSession = useOpenSession();
  const text = agent.fromTicket ? `${name} · ${made ? 'from ' : ''}${agent.fromTicket}` : name;
  const projectId = agent.projectId;
  if (!projectId) return <>{text}</>;
  return (
    <button
      type="button"
      title={`Open the ${name} session`}
      onClick={() => openSession({ id: agent.sessionId, projectId })}
      className="rounded-sm outline-none hover:text-primary hover:underline focus-visible:ring-2 focus-visible:ring-ring"
    >
      {text}
    </button>
  );
}

/** An outcome's summary and the workflow it came from. */
function outcomeBody(e: TicketEvent) {
  const d = (e.data ?? {}) as OutcomeData;
  if (!d.summary && !d.workflowName) return undefined;
  return (
    <div className="flex min-w-0 flex-col gap-1">
      {d.summary && <Markdown>{d.summary}</Markdown>}
      {d.workflowName && <span className="text-meta text-muted-foreground">from workflow {d.workflowName}</span>}
    </div>
  );
}

function toItem(e: TicketEvent): TimelineItem {
  const icon = iconFor(e);
  const outcome = e.kind === 'outcome' ? outcomeBody(e) : undefined;
  return {
    id: e.id,
    actor: actorOf(e),
    ...(e.actor === 'agent' && e.agent ? { actorLabel: <AgentLabel agent={e.agent} name={actorOf(e)} made={e.kind === 'created'} /> } : {}),
    // Your decision on the PM's proposal (PDT-475): "You · proposed by PM".
    ...(e.actor === 'user' && e.proposedBy
      ? { actorLabel: <>You · proposed by <AgentLabel agent={{ ...e.proposedBy, fromTicket: null }} name={e.proposedBy.persona ?? 'Agent'} /></> }
      : {}),
    time: e.createdAt,
    title: eventTitle(e),
    ...(icon ? { icon } : {}),
    ...(e.kind === 'comment' && e.body ? { body: <Markdown>{e.body}</Markdown> } : {}),
    ...(outcome ? { body: outcome } : {}),
    // What an outsider wrote, held back from the agent (PDT-202): shown here only.
    ...(e.kind === 'outside_input' && e.body
      ? {
          body: (
            <details>
              <summary className="cursor-pointer text-xs text-muted-foreground select-none">Show what they wrote</summary>
              <div className="mt-1"><Markdown>{e.body}</Markdown></div>
            </details>
          ),
        }
      : {}),
    // The review comments sent (they go with the worktree once it is removed; the timeline keeps them).
    ...(e.kind === 'feedback' && e.body
      ? {
          body: (
            <details>
              <summary className="cursor-pointer text-xs text-muted-foreground select-none">Show the comments</summary>
              <div className="mt-1"><Markdown>{e.body}</Markdown></div>
            </details>
          ),
        }
      : {}),
  };
}

/**
 * The ticket's activity, oldest first (newest last). Loads the newest page and "Show earlier" pages back; with
 * `recent` only the last few entries (the peek). `composer` adds a comment box under it.
 */
export function TicketTimeline({ ticketId, recent, composer = false }: { ticketId: string; recent?: number; composer?: boolean }) {
  const qc = useQueryClient();
  const full = useInfiniteQuery({
    queryKey: [...ticketKeys.events(ticketId), 'all'],
    queryFn: ({ pageParam }) => api.ticketEvents(ticketId, { newestFirst: true, limit: EVENT_PAGE, ...(pageParam ? { cursor: pageParam } : {}) }),
    initialPageParam: '' as string,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    refetchInterval: pagedPoll,
    enabled: recent === undefined,
  });
  const short = useQuery({
    queryKey: [...ticketKeys.events(ticketId), 'recent', recent],
    queryFn: () => api.ticketEvents(ticketId, { newestFirst: true, limit: recent }),
    refetchInterval: POLL,
    enabled: recent !== undefined,
  });
  const newestFirst = recent === undefined ? full.data?.pages.flatMap((p) => p.items) ?? [] : short.data?.items ?? [];
  const items = [...newestFirst].reverse().map(toItem);
  const loading = recent === undefined ? full.isPending : short.isPending;
  return (
    <OverviewSection title="Activity">
      {recent === undefined && full.hasNextPage && (
        <Button variant="ghost" size="sm" compact className="self-start" pending={full.isFetchingNextPage} onClick={() => void full.fetchNextPage()}>
          Show earlier
        </Button>
      )}
      {loading ? <p className="text-xs text-muted-foreground">Loading…</p> : <Timeline items={items} aria-label="Activity" />}
      {composer && (
        <CommentComposer ticketId={ticketId} />
      )}
    </OverviewSection>
  );
}

/**
 * The comment box: pasted, dropped or picked screenshots and logs are attached and shown in the comment (PDT-300). On
 * a phone the Attach button opens the photo library or files.
 */
export function CommentComposer({ ticketId, rows = 2, autoFocus = false, className = 'rounded-md border', onSent }: {
  ticketId: string;
  rows?: number;
  autoFocus?: boolean;
  className?: string;
  onSent?: () => void;
}) {
  const qc = useQueryClient();
  const { uploads, add, dismiss } = useAttachmentUploads(ticketId);
  return (
    <Composer
      label="Comment"
      sendLabel="Comment"
      submitOn="mod-enter"
      rows={rows}
      autoFocus={autoFocus}
      placeholder="Add a comment (Markdown; paste or drop screenshots and logs). ⌘/Ctrl+Enter sends."
      className={className}
      accept={ATTACH_ACCEPT}
      onFiles={async (files) => (await add(files)).map(attachmentMarkdown)}
      status={<UploadList uploads={uploads} onDismiss={dismiss} />}
      onSend={async (text) => {
        await api.commentOnTicket(ticketId, text);
        void qc.invalidateQueries({ queryKey: ticketKeys.events(ticketId) });
        onSent?.();
      }}
    />
  );
}

/** Label chips, for headers and rows. */
export function LabelChips({ labels, className }: { labels: string[]; className?: string }) {
  if (labels.length === 0) return null;
  return (
    <span className={cn('flex min-w-0 items-center gap-1', className)}>
      {labels.map((l) => <Chip key={l}>{l}</Chip>)}
    </span>
  );
}
