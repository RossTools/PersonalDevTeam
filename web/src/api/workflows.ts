import { enc, queryString, request } from './core';
import type { DocDiff } from './gen/DocDiff';
import type { DryRunRequest } from './gen/DryRunRequest';
import type { DryRunResult } from './gen/DryRunResult';
import type { GalleryTemplate } from './gen/GalleryTemplate';
import type { ImportCheck } from './gen/ImportCheck';
import type { ImportRequest } from './gen/ImportRequest';
import type { ImportResult } from './gen/ImportResult';
import type { Instantiate } from './gen/Instantiate';
import type { KindSpec } from './gen/KindSpec';
import type { NewWorkflow } from './gen/NewWorkflow';
import type { SaveWorkflow } from './gen/SaveWorkflow';
import type { Started } from './gen/Started';
import type { TemplateInput } from './gen/TemplateInput';
import type { Workflow } from './gen/Workflow';
import type { WorkflowCheck } from './gen/WorkflowCheck';
import type { WorkflowDetail } from './gen/WorkflowDetail';
import type { WorkflowDoc } from './gen/WorkflowDoc';
import type { WorkflowExport } from './gen/WorkflowExport';
import type { WorkflowPatch } from './gen/WorkflowPatch';
import type { WorkflowRun } from './gen/WorkflowRun';
import type { WorkflowRunDetail } from './gen/WorkflowRunDetail';
import type { WorkflowTemplate } from './gen/WorkflowTemplate';
import type { WorkflowVersion } from './gen/WorkflowVersion';

/** Workflows, their versions and runs, and workflow templates. */
export const workflowsApi = {
  listWorkflows: (project?: string) => request<Workflow[]>(`/api/workflows${queryString({ project })}`),
  patchWorkflow: (id: string, p: WorkflowPatch) =>
    request<Workflow>(`/api/workflows/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  listWorkflowRuns: (q: { workflow?: string; ticket?: string; status?: string; limit?: number } = {}) =>
    request<WorkflowRun[]>(`/api/workflow-runs${queryString(q)}`),
  workflowCatalog: () => request<KindSpec[]>('/api/workflows/catalog'),
  getWorkflow: (id: string) => request<WorkflowDetail>(`/api/workflows/${enc(id)}`),
  createWorkflow: (w: NewWorkflow) => request<WorkflowDetail>('/api/workflows', { method: 'POST', body: JSON.stringify(w) }),
  /** A new version; with `propose`, a proposal waiting for review. */
  saveWorkflow: (id: string, s: SaveWorkflow) =>
    request<WorkflowDetail>(`/api/workflows/${enc(id)}`, { method: 'PUT', body: JSON.stringify(s) }),
  deleteWorkflow: (id: string) => request<void>(`/api/workflows/${enc(id)}`, { method: 'DELETE' }),
  exportWorkflow: (id: string) => request<WorkflowExport>(`/api/workflows/${enc(id)}/export`),
  exportWorkflows: (ids: string[]) =>
    request<WorkflowExport>('/api/workflows/export', { method: 'POST', body: JSON.stringify({ ids }) }),
  checkImport: (text: string) =>
    request<ImportCheck>('/api/workflows/import', { method: 'POST', body: JSON.stringify({ text, check: true }) }),
  importWorkflows: (r: Omit<ImportRequest, 'check'>) =>
    request<ImportResult>('/api/workflows/import', { method: 'POST', body: JSON.stringify({ ...r, check: false }) }),
  checkWorkflow: (doc: WorkflowDoc) =>
    request<WorkflowCheck>('/api/workflows/check', { method: 'POST', body: JSON.stringify({ doc }) }),
  dryRunWorkflow: (r: DryRunRequest) =>
    request<DryRunResult>('/api/workflows/dry-run', { method: 'POST', body: JSON.stringify(r) }),
  runWorkflow: (id: string, input: unknown) =>
    request<Started>(`/api/workflows/${enc(id)}/run`, { method: 'POST', body: JSON.stringify({ input }) }),
  workflowVersions: (id: string) => request<WorkflowVersion[]>(`/api/workflows/${enc(id)}/versions`),
  workflowVersion: (id: string, v: number) => request<WorkflowVersion>(`/api/workflows/${enc(id)}/versions/${v}`),
  workflowDiff: (id: string, from: number, to: number) =>
    request<DocDiff>(`/api/workflows/${enc(id)}/diff${queryString({ from, to })}`),
  acceptWorkflowVersion: (id: string, v: number) =>
    request<WorkflowDetail>(`/api/workflows/${enc(id)}/versions/${v}/accept`, { method: 'POST' }),
  discardWorkflowVersion: (id: string, v: number) =>
    request<WorkflowDetail>(`/api/workflows/${enc(id)}/versions/${v}/discard`, { method: 'POST' }),
  getWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}`),
  retryWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}/retry`, { method: 'POST' }),
  resumeWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}/resume`, { method: 'POST' }),
  cancelWorkflowRun: (id: string) => request<WorkflowRunDetail>(`/api/workflow-runs/${enc(id)}/cancel`, { method: 'POST' }),
  answerWorkflowStep: (run: string, step: string, choice: string) =>
    request<WorkflowRunDetail>(`/api/workflow-runs/${enc(run)}/steps/${enc(step)}/answer`, { method: 'POST', body: JSON.stringify({ choice }) }),
  listWorkflowTemplates: () => request<GalleryTemplate[]>('/api/workflow-templates'),
  getWorkflowTemplate: (id: string) => request<WorkflowTemplate>(`/api/workflow-templates/${enc(id)}`),
  createWorkflowTemplate: (t: TemplateInput) =>
    request<WorkflowTemplate>('/api/workflow-templates', { method: 'POST', body: JSON.stringify(t) }),
  saveWorkflowTemplate: (id: string, t: TemplateInput) =>
    request<WorkflowTemplate>(`/api/workflow-templates/${enc(id)}`, { method: 'PUT', body: JSON.stringify(t) }),
  deleteWorkflowTemplate: (id: string) => request<void>(`/api/workflow-templates/${enc(id)}`, { method: 'DELETE' }),
  instantiateWorkflowTemplate: (id: string, i: Instantiate) =>
    request<WorkflowDetail>(`/api/workflow-templates/${enc(id)}/instantiate`, { method: 'POST', body: JSON.stringify(i) }),
};
