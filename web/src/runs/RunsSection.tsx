import { useQuery } from '@tanstack/react-query';
import { StatusDot } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { parseChoice, RunMenu, useRunChoices } from './RunMenu';
import { runDot, runLabel } from './runState';
import { useStartRun } from './useStartRun';

/** The runs of one place (main checkout or worktree) and a menu to start one from its configuration. */
export function RunsSection({ project, worktreeId, label, selectedRunId, onSelectRun }: {
  project: Project;
  worktreeId: string | null;
  label: string;
  selectedRunId: string | null;
  onSelectRun: (id: string) => void;
}) {
  const runs = useQuery({ queryKey: ['runs', project.id], queryFn: () => api.listRuns(project.id), refetchInterval: 2000 });
  const { choices, error } = useRunChoices(project.id, worktreeId);
  const start = useStartRun(project.id, onSelectRun);
  const mine = (runs.data ?? []).filter((r) => r.worktreeId === worktreeId);

  return (
    <div className="flex flex-col gap-0.5">
      {error && <p className="px-3 text-[11px] text-failed">{error}</p>}
      {choices.length > 0 && (
        <RunMenu
          label={`Run in ${label}`}
          choices={choices}
          value=""
          disabled={start.isPending}
          onChange={(c) => start.mutate({ ...(worktreeId ? { worktreeId } : {}), ...parseChoice(c) })}
          className="mx-3"
        />
      )}
      <ul className="flex flex-col gap-0.5">
        {mine.map((r) => (
          <li key={r.id}>
            <button
              type="button"
              aria-current={r.id === selectedRunId ? 'true' : undefined}
              onClick={() => onSelectRun(r.id)}
              className={`w-full flex items-center gap-2 text-left px-3 py-1 rounded-pdt text-sm ${r.id === selectedRunId ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
            >
              <StatusDot status={runDot(r.state)} />
              <span className="truncate">{r.kind === 'test' ? 'Test ' : ''}{r.name} · {runLabel(r.state)}</span>
            </button>
          </li>
        ))}
      </ul>
      {start.isError && <p role="alert" className="px-3 text-xs text-failed">{start.error.message}</p>}
    </div>
  );
}
