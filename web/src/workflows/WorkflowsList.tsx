// The Workflows view's list column: New workflow, Import and Export (PDT-155), and the workflows by
// where they apply; templates are in the New workflow gallery (PDT-229). On a phone there is no New workflow or Import
// (workflows are view-only there, PDT-120).
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { Badge, Button, List, ListHeading, ListRow, StatusDot } from '@ui';
import { api } from '../api/client';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import type { Project } from '../api/gen/Project';
import type { Workflow } from '../api/gen/Workflow';
import { ExportDialog } from './ExportDialog';
import { ImportDialog } from './ImportDialog';
import { scopeLabel } from './scope';
import type { WorkflowTab } from './WorkflowPage';
import { workflowKeys } from './queries';
import { POLL } from '../common/poll';

/** What the Workflows view shows: a workflow (on a tab, a run of it), a template, or nothing yet. */
export type WorkflowTarget = { kind: 'workflow'; id: string; tab?: WorkflowTab; runId?: string } | { kind: 'template'; id: string } | null;

/** Groups in order: this host, personal, then each project by name. */
export function groupWorkflows(list: Workflow[], projects: Project[]): { label: string; items: Workflow[] }[] {
  const groups = new Map<string, Workflow[]>();
  const order = (w: Workflow) => (w.scope === 'host' ? '0' : w.scope === 'personal' ? '1' : `2${scopeLabel(w, projects)}`);
  for (const w of [...list].sort((a, b) => order(a).localeCompare(order(b)))) {
    const label = scopeLabel(w, projects);
    groups.set(label, [...(groups.get(label) ?? []), w]);
  }
  return [...groups].map(([label, items]) => ({ label, items }));
}

export function WorkflowsList({ projects, current, onOpen, onNew }: {
  projects: Project[]; current: WorkflowTarget; onOpen: (t: WorkflowTarget) => void; onNew: () => void;
}) {
  const workflows = useQuery({ queryKey: workflowKeys.all, queryFn: () => api.listWorkflows(), refetchInterval: POLL.steady });
  const groups = groupWorkflows(workflows.data ?? [], projects);
  const phone = useMediaQuery(PHONE_QUERY);
  const [moving, setMoving] = useState<'import' | 'export' | null>(null);
  return (
    <nav aria-label="Workflows" className="flex flex-col gap-0.5 py-1">
      {!phone && (
        <div className="flex flex-col gap-1 px-2 pt-2">
          <Button size="sm" className="w-full" onClick={onNew}><Plus aria-hidden /> New workflow</Button>
          <div className="flex gap-1">
            <Button size="sm" variant="secondary" className="flex-1" onClick={() => setMoving('import')}>Import…</Button>
            <Button size="sm" variant="secondary" className="flex-1" disabled={!workflows.data?.length} onClick={() => setMoving('export')}>Export…</Button>
          </div>
        </div>
      )}
      {moving === 'import' && <ImportDialog projects={projects} onClose={() => setMoving(null)} onImported={(id) => onOpen({ kind: 'workflow', id })} />}
      {moving === 'export' && <ExportDialog workflows={groups.flatMap((g) => g.items)} onClose={() => setMoving(null)} />}
      {workflows.error && <p className="px-3 text-sm text-destructive">Workflows could not be loaded: {workflows.error.message}</p>}
      {groups.map((g) => (
        <section key={g.label} aria-label={g.label}>
          <ListHeading>{g.label}</ListHeading>
          <List aria-label={`Workflows: ${g.label}`} onAction={(id) => onOpen({ kind: 'workflow', id })}>
            {g.items.map((w) => (
              <ListRow
                key={w.id}
                id={w.id}
                title={w.name}
                textValue={w.name}
                current={current?.kind === 'workflow' && current.id === w.id}
                leading={<StatusDot status={w.enabled ? 'running' : 'idle'} label={w.enabled ? 'On' : 'Off'} />}
                trailing={
                  <span className="flex gap-1">
                    {w.proposed !== null && <Badge tone="waiting">Proposal waiting</Badge>}
                    {w.source === 'repo' && <Badge tone="neutral">Repo</Badge>}
                    {w.builtin && <Badge tone="neutral">Built in</Badge>}
                  </span>
                }
              />
            ))}
          </List>
        </section>
      ))}
      {workflows.isSuccess && groups.length === 0 && <p className="px-3 text-sm text-muted-foreground">No workflows yet.</p>}
    </nav>
  );
}
