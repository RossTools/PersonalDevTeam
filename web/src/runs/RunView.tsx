import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Alert, Button, Link, SegmentedControl, useConfirm, ViewHeader } from '@ui';
import { api } from '../api/client';
import type { Run } from '../api/gen/Run';
import { agentState, AgentDot } from '../common/AgentState';
import { TerminalView } from '../terminal/TerminalView';
import { runPtyUrl } from '../terminal/ptySocket';
import { isActive, runLabel } from './runState';
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
  const confirm = useConfirm();
  const refresh = () => qc.invalidateQueries({ queryKey: ['runs', run.projectId] });
  // The tab follows the run's state (Setup while setting up, else Output) unless the user picked one in this state.
  const [picked, setPicked] = useState<{ state: Run['state']; step: Step } | null>(null);
  const step: Step = picked?.state === run.state ? picked.step : run.state === 'setting_up' ? 'setup' : 'command';
  const stop = useMutation({ mutationFn: () => api.stopRun(run.id), onSettled: refresh });
  // Restarting counts toward the host's run limit, so it asks before stopping another run, like starting does.
  const restart = useMutation({
    mutationFn: () => withStopConfirm(confirm, (stopRunId) => api.restartRun(run.id, stopRunId)),
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
  const name = `${run.kind === 'test' ? 'Test ' : ''}${run.name}`;

  const askReset = async () => {
    const { ok } = await confirm({
      title: `Reset the data of ${name}?`,
      body: `Everything in its data folder${run.dataDir ? ` (${run.dataDir})` : ''} is deleted. The next start begins empty.`,
      action: 'Reset data',
      danger: true,
    });
    if (ok) reset.mutate();
  };
  const askRemove = async () => {
    const { ok } = await confirm({
      title: `Remove ${name} · ${run.branch}?`,
      body: 'Its setup and output logs are deleted. Its data folder and the checkout stay.',
      action: 'Remove run',
      danger: true,
    });
    if (ok) remove.mutate();
  };

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <ViewHeader
        leading={<AgentDot state={agentState({ kind: 'run', status: run.state })} />}
        title={`${name} · ${run.branch}`}
        titleTooltip={`${name} · ${run.branch}`}
        meta={
          <>
            {branchLink && <Link href={branchLink} external>GitHub</Link>}
            <span>{runLabel(run.state)}{run.exitCode !== null ? ` (exit ${run.exitCode})` : ''}</span>
            {run.url && run.state === 'ready' ? (
              <Link href={run.url} external className="min-w-0 truncate">{run.url}</Link>
            ) : (
              run.url && <span className="truncate">{run.url}</span>
            )}
          </>
        }
        actions={
          <>
            {active && <Button variant="ghost" size="sm" disabled={busy} pending={stop.isPending} onClick={() => stop.mutate()}>Stop</Button>}
            <Button variant="ghost" size="sm" disabled={busy} pending={restart.isPending} onClick={() => restart.mutate()}>Restart</Button>
            {!active && run.dataDir && (
              <Button variant="ghost" size="sm" disabled={busy} pending={reset.isPending} onClick={() => void askReset()}>Reset data</Button>
            )}
            {!active && <Button variant="ghost" size="sm" disabled={busy} pending={remove.isPending} onClick={() => void askRemove()}>Remove</Button>}
          </>
        }
      />
      {(run.error || error) && (
        <div className="flex flex-col gap-1 px-3 py-1.5">
          {run.error && <Alert tone="error">{run.error}</Alert>}
          {error && <Alert tone="error">{error.message}</Alert>}
        </div>
      )}
      <div className="flex border-b border-border px-2 py-1">
        <SegmentedControl<Step>
          aria-label="Run output"
          size="sm"
          options={[{ value: 'setup', label: 'Setup' }, { value: 'command', label: 'Output' }]}
          value={step}
          onChange={(s) => setPicked({ state: run.state, step: s })}
        />
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
