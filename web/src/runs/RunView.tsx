import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Alert, Button, Link, MenuButton, SegmentedControl, useConfirm, ViewHeader, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { Run } from '../api/gen/Run';
import { agentState, AgentDot } from '../common/AgentState';
import { BranchChip } from '../common/BranchChip';
import { copyToClipboard } from '../common/clipboard';
import { useAction } from '../common/useAction';
import { TerminalView } from '../terminal/TerminalView';
import { runPtyUrl } from '../terminal/ptySocket';
import { isActive, runLabel } from './runState';
import { withStopConfirm } from './useStartRun';

type Step = 'setup' | 'command';

/** Keyed by run id by its parent, so tab state never carries over from another run. */
export function RunView({ run, polledAt, onRemoved, branchLink, repo, baseBranch }: {
  run: Run;
  polledAt?: number;
  onRemoved: () => void;
  /** The run's branch on GitHub, when known. Superseded by the branch chip's menu when `repo` is known. */
  branchLink?: string | null;
  /** The project's `owner/repo` on GitHub (the branch chip's links); read from the projects list when left out. */
  repo?: string | null;
  /** The project's base branch (the branch chip's "Open PR"); read from the projects list when left out. */
  baseBranch?: string;
}) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const projects = useQuery({
    queryKey: ['projects'],
    queryFn: api.listProjects,
    enabled: repo === undefined || baseBranch === undefined,
    staleTime: 30_000,
  });
  const project = projects.data?.find((p) => p.id === run.projectId);
  const theRepo = repo !== undefined ? repo : project?.githubRepo ?? null;
  const theBase = baseBranch ?? project?.baseBranch;
  const refresh = () => qc.invalidateQueries({ queryKey: ['runs', run.projectId] });
  // The tab follows the run's state (Setup while setting up, else Output) unless the user picked one in this state.
  const [picked, setPicked] = useState<{ state: Run['state']; step: Step } | null>(null);
  const step: Step = picked?.state === run.state ? picked.step : run.state === 'setting_up' ? 'setup' : 'command';
  const [stop, stopping] = useAction(async () => {
    try { await api.stopRun(run.id); } finally { void refresh(); }
  });
  // Restarting counts toward the host's run limit, so it asks before stopping another run, like starting does.
  const [restart, restarting] = useAction(async () => {
    try { await withStopConfirm(confirm, (stopRunId) => api.restartRun(run.id, stopRunId)); } finally { void refresh(); }
  });
  const [reset, resetting] = useAction(async () => { await api.resetRunData(run.id); void refresh(); }, { success: 'Data reset' });
  const [remove, removing] = useAction(async () => {
    await api.deleteRun(run.id);
    void refresh();
    onRemoved();
  });
  const active = isActive(run.state);
  const busy = stopping || restarting || resetting || removing;
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
    if (ok) await reset();
  };
  const askRemove = async () => {
    const { ok } = await confirm({
      title: `Remove ${name} · ${run.branch}?`,
      body: 'Its setup and output logs are deleted. Its data folder and the checkout stay.',
      action: 'Remove run',
      danger: true,
    });
    if (ok) await remove();
  };

  const entries = (): MenuEntry[] => [
    ...(active ? [{ label: 'Restart', onSelect: () => void restart(), disabled: busy }] : []),
    ...(run.url ? [{ label: 'Copy URL', onSelect: () => void copyToClipboard(run.url!, 'URL copied') }] : []),
    ...(!theRepo && branchLink ? [{ label: 'Open on GitHub', onSelect: () => void window.open(branchLink, '_blank', 'noopener,noreferrer') }] : []),
    ...(!active
      ? [
          { separator: true } as const,
          ...(run.dataDir ? [{ label: 'Reset data…', onSelect: () => void askReset(), disabled: busy, danger: true }] : []),
          { label: 'Remove…', onSelect: () => void askRemove(), disabled: busy, danger: true },
        ]
      : []),
  ];

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <ViewHeader
        leading={<AgentDot state={agentState({ kind: 'run', status: run.state })} />}
        title={name}
        titleTooltip={`${name} · ${run.branch}`}
        meta={
          <>
            <BranchChip branch={run.branch} repo={theRepo} base={theBase} className="min-w-0" />
            <span className="shrink-0">{runLabel(run.state)}{run.exitCode !== null ? ` (exit ${run.exitCode})` : ''}</span>
            {run.url && run.state === 'ready' ? (
              <Link href={run.url} external className="min-w-0 truncate">{run.url}</Link>
            ) : (
              run.url && <span className="min-w-0 truncate">{run.url}</span>
            )}
          </>
        }
        actions={
          <>
            {active ? (
              <Button variant="primary" size="sm" disabled={busy} pending={stopping} onClick={() => void stop()}>Stop</Button>
            ) : (
              <Button variant="primary" size="sm" disabled={busy} pending={restarting} onClick={() => void restart()}>Restart</Button>
            )}
            <MenuButton label="Run actions" size="sm" placement="bottom end" entries={entries} />
          </>
        }
      />
      {run.error && <div className="px-3 py-1.5"><Alert tone="error">{run.error}</Alert></div>}
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
