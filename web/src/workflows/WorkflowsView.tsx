// The Workflows view's work area (Phase 7d spec §2): a workflow's page, one of the user's templates' page (from Save
// as template or the gallery's Edit this template), or a start.
import { useQuery } from '@tanstack/react-query';
import { Alert, Button, EmptyState, Skeleton } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { NewWorkflowDialog } from './NewWorkflowDialog';
import { TemplatePage } from './TemplatePage';
import { WorkflowPage } from './WorkflowPage';
import type { WorkflowTarget } from './WorkflowsList';

export function WorkflowsView({ target, onOpen, projects, creating, onCreate }: {
  target: WorkflowTarget;
  onOpen: (t: WorkflowTarget) => void;
  projects: Project[];
  /** The New workflow dialog is open (with a template picked). */
  creating: { templateId?: string } | null;
  onCreate: (c: { templateId?: string } | null) => void;
}) {
  const catalog = useQuery({ queryKey: ['workflow-catalog'], queryFn: () => api.workflowCatalog(), staleTime: Infinity });
  const body = catalog.error ? <Alert tone="error" className="m-3">The node catalog could not be loaded: {catalog.error.message}</Alert>
    : !catalog.data ? <Skeleton lines={8} className="p-4" />
      : target?.kind === 'workflow' ? <WorkflowPage key={target.id} id={target.id} catalog={catalog.data} projects={projects} onOpen={onOpen}
          view={{ tab: target.tab, runId: target.runId ?? null, onChange: (tab, runId) => onOpen({ kind: 'workflow', id: target.id, tab, ...(runId ? { runId } : {}) }) }} />
        : target?.kind === 'template' ? <TemplatePage key={target.id} id={target.id} catalog={catalog.data} onUse={(templateId) => onCreate({ templateId })} onDeleted={() => onOpen(null)} />
          : (
            <EmptyState action={<Button variant="primary" onClick={() => onCreate({})}>New workflow</Button>}>
              Workflows are what PDT does by itself when something happens. Pick one, or make one from a template.
            </EmptyState>
          );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {body}
      {creating && (
        <NewWorkflowDialog projects={projects} templateId={creating.templateId} onClose={() => onCreate(null)}
          onCreated={(id) => { onCreate(null); onOpen({ kind: 'workflow', id }); }}
          onEditTemplate={(id) => { onCreate(null); onOpen({ kind: 'template', id }); }} />
      )}
    </div>
  );
}
