// Accept and Discard for a version an agent proposed (PDT-320): the workflow's banner and its Versions tab share them,
// on desktop and on phones (deciding on a proposal is no editing).
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useConfirm } from '@ui';
import { api } from '../api/client';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';

/**
 * `dirty`: the editor has unsaved changes, which Accept (and Restore) replace, so they ask first. `onChanged` runs
 * with the workflow after either.
 */
export function useProposal(w: WorkflowDetail, dirty: boolean, onChanged: (d: WorkflowDetail) => void) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const done = (d: WorkflowDetail) => {
    qc.setQueryData(['workflow', w.id], d);
    void qc.invalidateQueries({ queryKey: ['workflows'] });
    void qc.invalidateQueries({ queryKey: ['workflow-versions', w.id] });
    void qc.invalidateQueries({ queryKey: ['inbox'] });
    onChanged(d);
  };
  /** Whether to go on and replace the editor's draft (it asks only when there are unsaved changes). */
  const replaceDraft = async (action: string) =>
    !dirty || (await confirm({ title: `${action} and lose your unsaved changes?`, body: 'The editor has changes that are not saved.', action, danger: true })).ok;
  const accept = useMutation({ mutationFn: (n: number) => api.acceptWorkflowVersion(w.id, n), onSuccess: done });
  const discard = useMutation({ mutationFn: (n: number) => api.discardWorkflowVersion(w.id, n), onSuccess: done });
  return {
    done,
    replaceDraft,
    accept: async (n: number) => { if (await replaceDraft('Accept')) accept.mutate(n); },
    discard: (n: number) => discard.mutate(n),
    accepting: accept.isPending,
    discarding: discard.isPending,
    error: accept.error ?? discard.error,
  };
}
