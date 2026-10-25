// Exports as text (PDT-155): the envelope pretty-printed, the same whether it is copied or downloaded.
import { api } from '../api/client';
import type { WorkflowExport } from '../api/gen/WorkflowExport';
import { copyToClipboard } from '../common/clipboard';
import { runAction } from '../common/useAction';

export const exportText = (e: WorkflowExport) => JSON.stringify(e, null, 2);

/** The export of these workflows (one: its own route). */
export const fetchExport = (ids: string[]) => (ids.length === 1 ? api.exportWorkflow(ids[0]) : api.exportWorkflows(ids));

/** A file name for the export: its one workflow's name, or `workflows`. */
export function exportFileName(e: WorkflowExport) {
  const base = e.workflows.length === 1 ? e.workflows[0].name : 'workflows';
  const slug = base.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'workflow';
  return `${slug}.pdt-workflow.json`;
}

/** Saves `text` as a file through the browser's download. */
export function downloadText(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

/** Copy export: the workflows' export on the clipboard, with a "Copied" toast. */
export async function copyExport(ids: string[]) {
  const e = await runAction(() => fetchExport(ids));
  if (e) await copyToClipboard(exportText(e), 'Copied');
}
