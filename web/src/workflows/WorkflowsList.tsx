// The Workflows view's list column (Phase 7d spec §2): New workflow, the workflows by where they apply, and the
// templates.
import { useQuery } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { Badge, Button, List, ListRow, StatusDot } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { Workflow } from '../api/gen/Workflow';
import { scopeLabel } from './scope';
import type { WorkflowTab } from './WorkflowPage';

/** What the Workflows view shows: a workflow (on a tab, a run of it), a template, or nothing yet. */
export type WorkflowTarget = { kind: 'workflow'; id: string; tab?: WorkflowTab; runId?: string } | { kind: 'template'; id: string } | null;

const heading = 'px-3 pt-3 pb-1 text-2xs uppercase tracking-wider text-muted-foreground';

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
  const workflows = useQuery({ queryKey: ['workflows'], queryFn: () => api.listWorkflows(), refetchInterval: 5000 });
  const templates = useQuery({ queryKey: ['workflow-templates'], queryFn: () => api.listWorkflowTemplates() });
  const groups = groupWorkflows(workflows.data ?? [], projects);
  return (
    <nav aria-label="Workflows" className="flex flex-col gap-0.5 py-1">
      <div className="px-2 pt-2">
        <Button size="sm" className="w-full" onClick={onNew}><Plus aria-hidden /> New workflow</Button>
      </div>
      {workflows.error && <p className="px-3 text-sm text-destructive">Workflows could not be loaded: {workflows.error.message}</p>}
      {groups.map((g) => (
        <section key={g.label} aria-label={g.label}>
          <h2 className={heading}>{g.label}</h2>
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
                    {w.proposed !== null && <Badge tone="waiting">Proposed</Badge>}
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
      <h2 className={heading}>Templates</h2>
      <List aria-label="Templates" onAction={(id) => onOpen({ kind: 'template', id })}>
        {(templates.data ?? []).map((t) => (
          <ListRow key={t.id} id={t.id} title={t.name} textValue={t.name}
            current={current?.kind === 'template' && current.id === t.id}
            trailing={t.builtin ? <Badge tone="neutral">Built in</Badge> : undefined} />
        ))}
      </List>
    </nav>
  );
}
