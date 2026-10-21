import { useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, Field, Input, SegmentedControl, Select, Textarea, toast, useConfirm } from '@ui';
import { api } from '../api/client';
import type { Priority } from '../api/gen/Priority';
import type { Project } from '../api/gen/Project';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketStage } from '../api/gen/TicketStage';
import { Markdown } from '../common/Markdown';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { actionError } from '../common/useAction';
import { AreasEditor } from './Areas';
import { LabelsEditor } from './LabelsEditor';
import { OPEN_STAGES } from './model';
import { invalidateTickets, useLabels } from './queries';

export interface NewTicketDialogProps {
  projects: Project[];
  /** The project it goes in (a picker shows when there's more than one project). */
  projectId: string | null;
  /** Typed in the quick-add row before Shift+Enter, or drafted by an agent (Make ticket). */
  title?: string;
  /** Drafted by an agent (Make ticket). */
  description?: string;
  /** A follow-up offered as a checkbox (off by default), run once the ticket exists: Make ticket's "Start from this chat". */
  then?: { label: string; hint?: string; run: (t: Ticket) => Promise<void> };
  /** The form's heading ("New ticket"). */
  heading?: string;
  onClose: () => void;
  onCreated?: (t: Ticket) => void;
}

/** The full new-ticket form: title, description (with preview), stage, priority, labels, areas. It lands at the top of its stage. */
export function NewTicketDialog({
  projects, projectId: initialProject, title: initialTitle = '', description: initialDescription = '', then, heading = 'New ticket', onClose, onCreated,
}: NewTicketDialogProps) {
  const qc = useQueryClient();
  const [projectId, setProjectId] = useState<string | null>(initialProject ?? projects[0]?.id ?? null);
  const [title, setTitle] = useState(initialTitle);
  const [description, setDescription] = useState(initialDescription);
  const [follow, setFollow] = useState(false);
  const [mode, setMode] = useState<'write' | 'preview'>('write');
  const [stage, setStage] = useState<TicketStage>('draft');
  const [priority, setPriority] = useState<Priority>('none');
  const [labels, setLabels] = useState<string[]>([]);
  const [areas, setAreas] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const errorRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);
  const known = useLabels(projectId);
  const ready = !!projectId && !!title.trim();
  const confirm = useConfirm();
  // Anything typed or chosen (a title from quick add counts: that row clears when the form opens).
  const edited = !!title.trim() || !!description.trim() || stage !== 'draft' || priority !== 'none' || labels.length > 0 || areas.length > 0;

  /** Cancel, Escape, the close button or a click outside: asks first when closing would lose what was typed. */
  const requestClose = async () => {
    if (edited) {
      const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const { ok } = await confirm({
        title: 'Discard this ticket?',
        body: 'What you typed in the new ticket form is lost.',
        action: 'Discard',
        cancel: 'Keep editing',
        danger: true,
      });
      if (!ok) {
        // The question opens outside the form's dialog, so focus doesn't come back to the form by itself.
        requestAnimationFrame(() => (back && back !== document.body ? back : titleRef.current)?.focus());
        return;
      }
    }
    onClose();
  };

  const submit = async () => {
    if (!ready || pending) return;
    setPending(true);
    setError(null);
    try {
      const t = await api.createTicket(projectId!, {
        title: title.trim(),
        ...(description.trim() ? { description } : {}),
        stage,
        priority,
        ...(labels.length > 0 ? { labels } : {}),
        ...(areas.length > 0 ? { areas } : {}),
      });
      invalidateTickets(qc);
      if (then && follow) {
        try {
          await then.run(t);
        } catch (e) {
          // The ticket exists; only the follow-up failed.
          toast.error(`${t.key} was created, but: ${actionError(e)}`);
        }
      }
      onCreated?.(t);
      onClose();
    } catch (e) {
      setError(actionError(e));
      requestAnimationFrame(() => errorRef.current?.focus());
    } finally {
      setPending(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) void requestClose(); }}
      title={heading}
      size="lg"
      footer={
        <>
          <Button variant="secondary" onClick={() => void requestClose()}>Cancel</Button>
          <Button type="submit" form="new-ticket" variant="primary" pending={pending} disabled={!ready}>Create ticket</Button>
        </>
      }
    >
      {/* Three blocks divided by rules: what it is, the description, then its details. */}
      <form id="new-ticket" className="flex flex-col divide-y divide-border *:py-4 *:first:pt-0 *:last:pb-0" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
        <div className="flex flex-col gap-3">
          {projects.length > 1 && (
            <Field label="Project">
              <Select
                options={projects.map((p) => ({ value: p.id, label: `${p.key} · ${p.name}` }))}
                value={projectId}
                onChange={(v) => { setProjectId(v); setLabels([]); setAreas([]); }}
              />
            </Field>
          )}
          <Field label="Title">
            <Input ref={titleRef} autoFocus value={title} maxLength={200} onChange={(e) => setTitle(e.target.value)} placeholder="What needs doing" />
          </Field>
        </div>
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-2">
            <span id="new-ticket-desc" className="flex-1 text-xs font-medium">Description</span>
            <SegmentedControl
              aria-label="Description editor"
              size="sm"
              options={[{ value: 'write', label: 'Write' }, { value: 'preview', label: 'Preview' }]}
              value={mode}
              onChange={setMode}
            />
          </div>
          {mode === 'write' ? (
            <Textarea
              aria-labelledby="new-ticket-desc"
              rows={6}
              value={description}
              placeholder="Markdown: what and why, acceptance criteria…"
              onChange={(e) => setDescription(e.target.value)}
              className="min-h-28 resize-y"
            />
          ) : (
            <div className="min-h-28 rounded-md border border-border p-2">
              {description.trim() ? <Markdown breaks>{description}</Markdown> : <p className="text-xs text-muted-foreground">Nothing to preview.</p>}
            </div>
          )}
        </div>
        <div className="flex flex-col gap-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Stage">
              <Select options={OPEN_STAGES.map((s) => ({ value: s, label: STAGE_LABEL[s] }))} value={stage} onChange={setStage} />
            </Field>
            <Field label="Priority">
              <Select options={PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }))} value={priority} onChange={setPriority} />
            </Field>
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="text-xs font-medium">Labels</span>
            <LabelsEditor labels={labels} suggestions={known.data ?? []} onChange={setLabels} />
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="text-xs font-medium">Areas</span>
            <AreasEditor projectId={projectId} areas={areas} onChange={setAreas} />
          </div>
        </div>
        {then && (
          <div className="flex flex-col gap-1">
            <Checkbox checked={follow} onChange={setFollow}>{then.label}</Checkbox>
            {then.hint && <p className="pl-6 text-meta text-muted-foreground">{then.hint}</p>}
          </div>
        )}
        {error && <div ref={errorRef} tabIndex={-1} className="outline-none"><Alert tone="error">{error}</Alert></div>}
      </form>
    </Dialog>
  );
}
