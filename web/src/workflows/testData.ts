// Test fixtures: the real catalog (GET /api/workflows/catalog from this branch's pdtd) and a small workflow.
import type { KindSpec } from '../api/gen/KindSpec';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import catalog from './catalog.fixture.json';

export const CATALOG = catalog as unknown as KindSpec[];

export const DOC: WorkflowDoc = {
  name: 'CI to agent',
  description: 'Tells the agent',
  nodes: [
    { id: 'ci', kind: 'ci.run_failed', version: 1, config: {}, position: { x: 0, y: 0 } },
    { id: 'tell', kind: 'session.send', version: 1, config: { attempt: '{{trigger.attemptId}}', text: 'CI failed' }, position: { x: 300, y: 0 } },
  ],
  edges: [{ from: 'ci', port: 'out', to: 'tell' }],
  settings: { maxSteps: 200, maxRuntimeMs: 604800000 },
} as WorkflowDoc;

export const detail = (over: Partial<WorkflowDetail> = {}): WorkflowDetail => ({
  id: 'w1', name: DOC.name, description: DOC.description, scope: 'host', projectId: null, enabled: false, builtin: null,
  source: 'host', repoFile: null, version: 1, proposed: null, createdAt: 1, updatedAt: 1, doc: DOC, issues: [], needsProject: false, ...over,
});
