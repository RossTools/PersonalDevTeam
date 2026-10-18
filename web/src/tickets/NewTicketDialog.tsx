import { useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Field, Input, SegmentedControl, Select, Textarea } from '@ui';
import { api } from '../api/client';
import type { Priority } from '../api/gen/Priority';
import type { Project } from '../api/gen/Project';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketStage } from '../api/gen/TicketStage';
import { Markdown } from '../common/Markdown';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { actionError } from '../common/useAction';
import { LabelsEditor } from './LabelsEditor';
import { OPEN_STAGES } from './model';
import { invalidateTickets, useLabels } from './queries';

export interface NewTicketDialogProps {
  projects: Project[];
  /** The project it goes in (a picker shows when there's more than one project). */
  projectId: string | null;
  /** Typed in the quick-add row before Shift+Enter. */
  title?: string;
  onClose: () => void;
  onCreated?: (t: Ticket) => void;
}

/** The full new-ticket form: title, description (with preview), stage, priority, labels. It lands at the top of its stage. */
export function NewTicketDialog({ projects, projectId: initialProject, title: initialTitle = '', onClose, onCreated }: NewTicketDialogProps) {
  const qc = useQueryClient();
  const [projectId, setProjectId] = useState<string | null>(initialProject ?? projects[0]?.id ?? null);
  const [title, setTitle] = useState(initialTitle);
  const [description, setDescription] = useState('');
  const [mode, setMode] = useState<'write' | 'preview'>('write');
  const [stage, setStage] = useState<TicketStage>('draft');
  const [priority, setPriority] = useState<Priority>('none');
  const [labels, setLabels] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const errorRef = useRef<HTMLDivElement>(null);
  const known = useLabels(projectId);
  const ready = !!projectId && !!title.trim();

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
      });
      invalidateTickets(qc);
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
      onOpenChange={(o) => !o && onClose()}
      title="New ticket"
      size="lg"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          <Button type="submit" form="new-ticket" variant="primary" pending={pending} disabled={!ready}>Create ticket</Button>
        </>
      }
    >
      <form id="new-ticket" className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
        {projects.length > 1 && (
          <Field label="Project">
            <Select
              options={projects.map((p) => ({ value: p.id, label: `${p.key} · ${p.name}` }))}
              value={projectId}
              onChange={(v) => { setProjectId(v); setLabels([]); }}
            />
          </Field>
        )}
        <Field label="Title">
          <Input autoFocus value={title} maxLength={200} onChange={(e) => setTitle(e.target.value)} placeholder="What needs doing" />
        </Field>
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
              {description.trim() ? <Markdown>{description}</Markdown> : <p className="text-xs text-muted-foreground">Nothing to preview.</p>}
            </div>
          )}
        </div>
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
        {error && <div ref={errorRef} tabIndex={-1} className="outline-none"><Alert tone="error">{error}</Alert></div>}
      </form>
    </Dialog>
  );
}
