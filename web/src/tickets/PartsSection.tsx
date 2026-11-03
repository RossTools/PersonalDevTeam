// Ticket parts (PDT-38): a parent's Parts section (stepper, ordered parts, quick add, Start next part), the parent chip
// a part shows, the Parent field, and "Make into parts…".
import { useState, type DragEvent, type KeyboardEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { GripVertical, Play, Plus, X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Alert, Badge, Button, Checkbox, Dialog, dragData, IconButton, Input, parseDragData, toast } from '@ui';
import { api } from '../api/client';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketParent } from '../api/gen/TicketParent';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { PartsStepper } from '../common/PartsStepper';
import { STAGE_LABEL } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { actionError, runAction } from '../common/useAction';
import { patchTicket, startNextPart, useOpenTicket, useStartTicket } from './actions';
import { isClosed, nextPart, scopeParts } from './model';
import { invalidateTickets } from './queries';
import { PrBadge } from './PullRequest';
import { OverviewSection } from './TicketParts';
import { TicketPicker } from './TicketPicker';
import { plural } from '../common/text';

const PART_DRAG_KIND = 'part';

/** A part's parent as a chip (`PDT-2 ›`) that opens it; plain text with `static` (inside a clickable row). */
export function ParentChip({ parent, projectId, static: still = false, className }: {
  parent: TicketParent;
  projectId: string;
  static?: boolean;
  className?: string;
}) {
  const open = useOpenTicket();
  return (
    <TicketKey
      ticketKey={`${parent.key} ›`}
      title={`Part of ${parent.key}: ${parent.title}`}
      {...(still ? {} : { onClick: () => open({ id: parent.id, projectId }) })}
      className={className}
    />
  );
}

/** "Make into parts…" is offered for an open ticket that is not a part and has no active attempt. */
export const canMakeParts = (t: Pick<Ticket, 'parent' | 'attempt' | 'stage'>) =>
  !t.parent && t.attempt?.state !== 'active' && !isClosed(t.stage);

function PartRow({ part, index, prev, next, projectId }: { part: TicketSummary; index: number; prev?: TicketSummary; next?: TicketSummary; projectId: string }) {
  const qc = useQueryClient();
  const open = useOpenTicket();
  const [over, setOver] = useState(false);
  const rank = (m: { before: string } | { after: string }) =>
    void runAction(async () => {
      await api.rankTicket(part.id, m);
      invalidateTickets(qc, [part.id]);
    });
  const onKeyDown = (e: KeyboardEvent) => {
    if (!e.altKey) return;
    if (e.key === 'ArrowUp' && prev) { e.preventDefault(); rank({ before: prev.id }); }
    if (e.key === 'ArrowDown' && next) { e.preventDefault(); rank({ after: next.id }); }
  };
  const onDrop = (e: DragEvent) => {
    setOver(false);
    const d = parseDragData(e.dataTransfer.getData('text/plain') ?? '');
    if (!d || d.kind !== PART_DRAG_KIND || d.id === part.id) return;
    e.preventDefault();
    const from = Number(e.dataTransfer.getData('application/x-pdt-part-index') || '-1');
    const here = Number((e.currentTarget as HTMLElement).dataset.index);
    void runAction(async () => {
      await api.rankTicket(d.id, from >= 0 && from < here ? { after: part.id } : { before: part.id });
      invalidateTickets(qc, [d.id]);
    });
  };
  const closed = isClosed(part.stage);
  return (
    <li
      aria-label={`${part.key} ${part.title}`}
      data-index={index}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData('text/plain', dragData(PART_DRAG_KIND, part.id));
        e.dataTransfer.setData('application/x-pdt-part-index', (e.currentTarget as HTMLElement).dataset.index ?? '-1');
        e.dataTransfer.effectAllowed = 'move';
      }}
      onDragOver={(e) => { e.preventDefault(); if (!over) setOver(true); }}
      onDragLeave={() => setOver(false)}
      onDrop={onDrop}
      onKeyDown={onKeyDown}
      className={cn('group/part flex min-w-0 items-center gap-2 rounded-md px-1 py-0.5 text-xs', over && 'bg-accent')}
    >
      <GripVertical aria-hidden className="size-3 shrink-0 cursor-grab text-muted-foreground pointer-coarse:hidden" />
      <TicketKey ticketKey={part.key} title={`${part.title} (Alt+↑/↓ to reorder)`} onClick={() => open({ id: part.id, projectId })} />
      <span className={cn('min-w-0 flex-1 truncate', closed && 'text-muted-foreground line-through')}>{part.title}</span>
      {part.attempt?.pr ? <PrBadge pr={part.attempt.pr} attemptMerged={part.attempt.state === 'merged'} /> : <Badge>{STAGE_LABEL[part.stage]}</Badge>}
    </li>
  );
}

/** The quick add under the parts: Enter adds a part (after the last one, blocked by it when ticked). */
function AddPart({ ticket, last }: { ticket: Ticket; last?: TicketSummary }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState('');
  const [after, setAfter] = useState(true);
  const [pending, setPending] = useState(false);
  const add = async () => {
    const t = title.trim();
    if (!t || pending) return;
    setPending(true);
    const made = await runAction(() =>
      api.createTicket(ticket.projectId, { title: t, parent: ticket.id, ...(last && after ? { blockedByPrevious: true } : {}) }),
    );
    setPending(false);
    if (made) {
      setTitle('');
      invalidateTickets(qc, [ticket.id]);
    }
  };
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <Plus aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        <Input
          aria-label="New part title"
          placeholder="New part… Enter to add"
          value={title}
          size="sm"
          maxLength={200}
          disabled={pending}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) { e.preventDefault(); void add(); }
          }}
        />
      </div>
      {last && (
        <Checkbox checked={after} onChange={setAfter} className="pl-6 text-meta">
          Starts after {last.key}
        </Checkbox>
      )}
    </div>
  );
}

/**
 * A parent's parts: the stepper, its parts in order (drag or Alt+↑/↓ to reorder), a quick add and Start next part.
 * Nothing for a ticket without parts (its ⋯ menu has Make into parts…).
 */
export function PartsSection({ ticket: t }: { ticket: Ticket }) {
  const start = useStartTicket();
  const parts = t.partList;
  if (parts.length === 0) return null;
  const next = nextPart(parts);
  return (
    <OverviewSection
      title="Parts"
      actions={
        !isClosed(t.stage) && parts.some((p) => !isClosed(p.stage)) && (
          <Button
            variant="ghost"
            size="sm"
            compact
            title={typeof next === 'string' ? `Nothing to start: ${next}` : `Start ${next.key}`}
            onClick={() => void runAction(() => startNextPart(start, t))}
          >
            <Play aria-hidden /> Start next part
          </Button>
        )
      }
    >
      {t.parts && <PartsStepper parts={t.parts} showCount />}
      <ol aria-label="Parts" className="flex flex-col gap-0.5">
        {parts.map((p, i) => (
          <PartRow key={p.id} part={p} index={i} prev={parts[i - 1]} next={parts[i + 1]} projectId={t.projectId} />
        ))}
      </ol>
      {!isClosed(t.stage) && <AddPart ticket={t} last={parts[parts.length - 1]} />}
    </OverviewSection>
  );
}

/** One row per part, prefilled from the description's Scope/Parts list; makes them all at once. */
export function MakePartsDialog({ ticket, onClose }: { ticket: Pick<Ticket, 'id' | 'key' | 'description'>; onClose: () => void }) {
  const qc = useQueryClient();
  const [rows, setRows] = useState<string[]>(() => {
    const found = scopeParts(ticket.description);
    return found.length > 0 ? found : ['', ''];
  });
  const [chain, setChain] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const titles = rows.map((r) => r.trim()).filter(Boolean);
  const submit = async () => {
    if (titles.length === 0 || pending) return;
    setPending(true);
    setError(null);
    try {
      await api.createParts(ticket.id, { parts: titles.map((title) => ({ title })), blockedByPrevious: chain });
      invalidateTickets(qc, [ticket.id]);
      toast.success(`${ticket.key} has ${plural(titles.length, 'new part')}.`);
      onClose();
    } catch (e) {
      setError(actionError(e));
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title={`Make ${ticket.key} into parts`}
      description="Each part is a ticket with its own branch and PR."
      size="md"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" pending={pending} disabled={titles.length === 0} onClick={() => void submit()}>
            {titles.length <= 1 ? 'Make part' : `Make ${titles.length} parts`}
          </Button>
        </>
      }
    >
      <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
        <ol aria-label="Parts to make" className="flex flex-col gap-1.5">
          {rows.map((r, i) => (
            <li key={i} className="flex items-center gap-2">
              <span className="w-5 shrink-0 text-right text-meta text-muted-foreground tabular-nums">{i + 1}</span>
              <Input
                aria-label={`Part ${i + 1} title`}
                value={r}
                size="sm"
                maxLength={200}
                onChange={(e) => setRows((list) => list.map((x, j) => (j === i ? e.target.value : x)))}
              />
              <IconButton label={`Remove part ${i + 1}`} size="sm" onClick={() => setRows((list) => list.filter((_, j) => j !== i))}>
                <X aria-hidden />
              </IconButton>
            </li>
          ))}
        </ol>
        <Button variant="ghost" size="sm" compact className="self-start" onClick={() => setRows((list) => [...list, ''])}>
          <Plus aria-hidden /> Add part
        </Button>
        <Checkbox checked={chain} onChange={setChain}>Each part starts after the previous one</Checkbox>
        {error && <Alert tone="error">{error}</Alert>}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

/** The Parent row of a ticket's details: the parent chip with Change… and Detach, or Set…. Not for a parent. */
export function ParentField({ ticket }: { ticket: Ticket }) {
  const qc = useQueryClient();
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = async (parent: string) => {
    setError(null);
    try {
      await patchTicket(qc, ticket.id, { parent });
      if (ticket.parent) invalidateTickets(qc, [ticket.parent.id]);
      if (parent) invalidateTickets(qc, [parent]);
      setPicking(false);
    } catch (e) {
      setError(actionError(e));
      if (!picking) toast.error(actionError(e));
    }
  };
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1">
      {ticket.parent ? <ParentChip parent={ticket.parent} projectId={ticket.projectId} /> : <span className="text-muted-foreground">None</span>}
      <Button variant="ghost" size="sm" compact onClick={() => { setError(null); setPicking(true); }}>
        {ticket.parent ? 'Change…' : 'Set…'}
      </Button>
      {ticket.parent && <Button variant="ghost" size="sm" compact onClick={() => void set('')}>Detach</Button>}
      {picking && (
        <Dialog open onOpenChange={(o) => !o && setPicking(false)} title={`Make ${ticket.key} a part of…`} size="md">
          <div className="flex flex-col gap-2">
            <TicketPicker
              projectId={ticket.projectId}
              exclude={[ticket.id, ...(ticket.parent ? [ticket.parent.id] : [])]}
              query={{ parent: 'none' }}
              filter={(t) => !isClosed(t.stage)}
              label="Parent ticket"
              onPick={(t) => void set(t.id)}
              className="max-h-[50dvh]"
            />
            {error && <Alert tone="error">{error}</Alert>}
          </div>
        </Dialog>
      )}
    </div>
  );
}
