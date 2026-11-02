import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useConfirm, type ConfirmOptions, type ConfirmResult } from '@ui';
import { api, ApiError } from '../api/client';
import type { NewRun } from '../api/gen/NewRun';
import type { Run } from '../api/gen/Run';

type Confirm = (o: ConfirmOptions) => Promise<ConfirmResult>;

/** Runs `attempt`. At the host's run limit it asks (the confirm dialog, with the server's message) before stopping the
 * oldest active run and trying again with that run's id; declining resolves to null without an error. */
export async function withStopConfirm<T>(confirm: Confirm, attempt: (stopRunId?: string) => Promise<T>): Promise<T | null> {
  try {
    return await attempt();
  } catch (e) {
    if (!(e instanceof ApiError) || e.code !== 'too_many_runs') throw e;
    const { ok } = await confirm({
      title: e.message,
      body: 'The host is running as many runs as it allows. The oldest run is stopped to make room.',
      action: 'Stop it and start',
    });
    if (!ok) return null;
    const oldest = (await api.listActiveRuns()).find((x) => x.kind === 'run');
    if (!oldest) throw e;
    return attempt(oldest.id);
  }
}

/** Starts a run, asking before it stops another one when the host is at its limit. Callers show its error. */
export function useStartRun(projectId: string, onStarted: (runId: string) => void) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  return useMutation({
    meta: { errorShown: true },
    mutationFn: (r: NewRun): Promise<Run | null> =>
      withStopConfirm(confirm, (stopRunId) => api.startRun(projectId, stopRunId ? { ...r, stopRunId } : r)),
    onSuccess: (run) => {
      void qc.invalidateQueries({ queryKey: ['runs', projectId] });
      void qc.invalidateQueries({ queryKey: ['worktrees', projectId] });
      if (run) onStarted(run.id);
    },
  });
}
