import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { Run } from '../api/gen/Run';
import { TerminalView } from '../terminal/TerminalView';
import { runPtyUrl } from '../terminal/ptySocket';
import { isActive, runDot, runLabel } from './runState';
import { withStopConfirm } from './useStartRun';

export function RunView({ run, polledAt, onRemoved }: { run: Run; polledAt?: number; onRemoved: () => void }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ['runs', run.projectId] });
  const [step, setStep] = useState<'setup' | 'command'>(run.state === 'setting_up' ? 'setup' : 'command');
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
  const tab = (s: 'setup' | 'command', label: string) => (
    <button
      type="button"
      role="tab"
      aria-selected={step === s}
      onClick={() => setStep(s)}
      className={`rounded-pdt px-2 py-0.5 ${step === s ? 'bg-accent-soft text-fg' : 'text-muted hover:bg-surface-2'}`}
    >
      {label}
    </button>
  );

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1.5 text-sm">
        <StatusDot status={runDot(run.state)} />
        <span className="font-medium">{run.kind === 'test' ? 'Test ' : ''}{run.name} · {run.branch}</span>
        <span className="text-xs text-muted">{runLabel(run.state)}{run.exitCode !== null ? ` (exit ${run.exitCode})` : ''}</span>
        {run.url && run.state === 'ready' ? (
          <a href={run.url} target="_blank" rel="noreferrer" className="text-xs text-accent underline">{run.url}</a>
        ) : (
          run.url && <span className="text-xs text-muted">{run.url}</span>
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
      <div role="tablist" aria-label="Run output" className="flex gap-1 px-2 py-1 border-b border-border text-xs">
        {tab('setup', 'Setup')}
        {tab('command', 'Output')}
      </div>
      <TerminalView
        key={`${run.id}:${step}`}
        sessionId={`${run.id}:${step}`}
        url={runPtyUrl(run.id, step)}
        sessionStatus={active ? 'running' : 'finished'}
        polledAt={polledAt}
      />
    </div>
  );
}
