// The Workflows view's list column: New workflow, with Import and Export (PDT-155) under its ⋯, and the workflows by
// where they apply; templates are in the New workflow gallery (PDT-229). On a phone there is no New workflow or Import
// (workflows are view-only there, PDT-120).
import { lazy, Suspense, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Badge, EmptyState, List, ListHeading, ListRow, MenuButton, StatusDot } from '@ui';
import { api } from '../api/client';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import type { Project } from '../api/gen/Project';
import type { Workflow } from '../api/gen/Workflow';
import { ExportDialog } from './ExportDialog';
import { scopeLabel } from './scope';
import type { WorkflowTab } from './WorkflowPage';
import { workflowKeys } from './queries';
import { POLL } from '../common/poll';
import { LoadError } from '../common/QueryStatus';
import { ListColumnHeader } from '../common/ListColumnHeader';

// Import is loaded the first time it opens, not with the app.
const ImportDialog = lazy(() => import('./ImportDialog').then((m) => ({ default: m.ImportDialog })));

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

/** A row's badges: a proposal waiting, one from the repo, one of the user's own. None for a built-in. */
function badges(w: Workflow) {
  const list = [
    w.proposed !== null && <Badge key="proposal" tone="waiting">Proposal waiting</Badge>,
    w.source === 'repo' && <Badge key="repo" tone="neutral">Repo</Badge>,
    !w.builtin && w.source !== 'repo' && <Badge key="yours" tone="neutral">Yours</Badge>,
  ].filter(Boolean);
  return list.length ? <span className="flex gap-1">{list}</span> : undefined;
}

export function WorkflowsList({ projects, current, onOpen, onNew }: {
  projects: Project[]; current: WorkflowTarget; onOpen: (t: WorkflowTarget) => void; onNew: () => void;
}) {
  const workflows = useQuery({ queryKey: workflowKeys.all, queryFn: () => api.listWorkflows(), refetchInterval: POLL.steady });
  const groups = groupWorkflows(workflows.data ?? [], projects);
  const phone = useMediaQuery(PHONE_QUERY);
  const [moving, setMoving] = useState<'import' | 'export' | null>(null);
  return (
    <nav aria-label="Workflows" className="flex flex-col gap-0.5 pb-1">
      {!phone && (
        <ListColumnHeader
          title="Workflows"
          newLabel="New workflow"
          onNew={onNew}
          extra={(
            <MenuButton
              label="Import or export workflows"
              size="sm"
              entries={() => [
                { label: 'Import…', onSelect: () => setMoving('import') },
                { label: 'Export…', onSelect: () => setMoving('export'), disabled: !workflows.data?.length },
              ]}
            />
          )}
        />
      )}
      {moving === 'import' && <Suspense fallback={null}><ImportDialog projects={projects} onClose={() => setMoving(null)} onImported={(id) => onOpen({ kind: 'workflow', id })} /></Suspense>}
      {moving === 'export' && <ExportDialog workflows={groups.flatMap((g) => g.items)} onClose={() => setMoving(null)} />}
      <LoadError what="Workflows" error={workflows.error} className="mx-3" />
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
                // On is a dot and Off a muted word in the same slot, so the off ones scan as a group; only the exceptions
                // get a badge, not the built-ins every host has (PDT-680).
                leading={
                  <span className="flex w-6 justify-center">
                    {w.enabled ? <StatusDot status="running" label="On" /> : <span className="text-xs text-muted-foreground">Off</span>}
                  </span>
                }
                trailing={badges(w)}
              />
            ))}
          </List>
        </section>
      ))}
      {workflows.isSuccess && groups.length === 0 && <EmptyState size="inline" className="px-3">No workflows yet.</EmptyState>}
      {phone && <p className="px-3 pt-2 text-xs text-muted-foreground">Create and edit workflows on a computer.</p>}
    </nav>
  );
}
