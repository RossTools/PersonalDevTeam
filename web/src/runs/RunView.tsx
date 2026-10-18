import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { Run } from '../api/gen/Run';
import { TerminalView } from '../terminal/TerminalView';
import { runPtyUrl } from '../terminal/ptySocket';
import { isActive, runDot, runLabel } from './runState';
import { withStopConfirm } from './useStartRun';

type Step = 'setup' | 'command';

/** Keyed by run id by its parent, so tab and error state never carry over from another run. */
export function RunView({ run, polledAt, onRemoved, branchLink }: {
  run: Run;
  polledAt?: number;
  onRemoved: () => void;
  /** The run's branch on GitHub, when known. */
  branchLink?: string | null;
}) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ['runs', run.projectId] });
  // The tab follows the run's state (Setup while setting up, else Output) unless the user picked one in this state.
  const [picked, setPicked] = useState<{ state: Run['state']; step: Step } | null>(null);
  const step: Step = picked?.state === run.state ? picked.step : run.state === 'setting_up' ? 'setup' : 'command';
  const stop = useMutation({ mutationFn: () => api.stopRun(run.id), onSettled: refresh });
  // Restarting counts toward the host's run limit, so it asks before stopping another run, like starting does.
  const restart = useMutation({
    mutationFn: () => withStopConfirm((stopRunId) => api.restartRun(run.id, stopRunId)),
    onSettled: refresh,
  });
  const reset = useMutation({ mutationFn: () => api.resetRunData(run.id), onSuccess: refresh });
  const remove = useMutation({
    mutationFn: () => api.deleteRun(run.id),
    onSuccess: () => {
      void refresh();
      onRemoved();
    },
  });
  const active = isActive(run.state);
  const busy = stop.isPending || restart.isPending || reset.isPending || remove.isPending;
  const error = stop.error ?? restart.error ?? reset.error ?? remove.error;
  // Only the step that is running has a live process; a finished step must not be reattached by the safety net.
  const stepLive = step === 'setup' ? run.state === 'setting_up' : ['starting', 'ready', 'running'].includes(run.state);
  const tab = (s: Step, label: string) => (
    <button
      type="button"
      aria-pressed={step === s}
      onClick={() => setPicked({ state: run.state, step: s })}
      className={`rounded-pdt px-2 py-0.5 ${step === s ? 'bg-accent text-fg' : 'text-muted-foreground hover:bg-surface-2'}`}
    >
      {label}
    </button>
  );

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1.5 text-sm">
        <StatusDot status={runDot(run.state)} />
        <span className="font-medium">{run.kind === 'test' ? 'Test ' : ''}{run.name} · {run.branch}</span>
        {branchLink && <a href={branchLink} target="_blank" rel="noreferrer" className="text-xs text-primary underline">GitHub</a>}
        <span className="text-xs text-muted-foreground">{runLabel(run.state)}{run.exitCode !== null ? ` (exit ${run.exitCode})` : ''}</span>
        {run.url && run.state === 'ready' ? (
          <a href={run.url} target="_blank" rel="noreferrer" className="text-xs text-primary underline">{run.url}</a>
        ) : (
          run.url && <span className="text-xs text-muted-foreground">{run.url}</span>
        )}
        <span className="flex-1" />
        {active && <Button variant="ghost" disabled={busy} onClick={() => stop.mutate()}>Stop</Button>}
        <Button variant="ghost" disabled={busy} onClick={() => restart.mutate()}>Restart</Button>
        {!active && run.dataDir && (
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => { if (window.confirm('Delete this run’s data folder contents?')) reset.mutate(); }}
          >
            Reset data
          </Button>
        )}
        {!active && <Button variant="ghost" disabled={busy} onClick={() => remove.mutate()}>Remove</Button>}
      </header>
      {run.error && <p role="alert" className="px-3 py-1 text-xs text-failed">{run.error}</p>}
      {error && <p role="alert" className="px-3 py-1 text-xs text-failed">{error.message}</p>}
      <div role="group" aria-label="Run output" className="flex gap-1 px-2 py-1 border-b border-border text-xs">
        {tab('setup', 'Setup')}
        {tab('command', 'Output')}
      </div>
      <TerminalView
        key={`${run.id}:${step}`}
        sessionId={`${run.id}:${step}`}
        url={runPtyUrl(run.id, step)}
        sessionStatus={stepLive ? 'running' : 'finished'}
        polledAt={polledAt}
      />
    </div>
  );
}
