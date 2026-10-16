import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { parseChoice, RunMenu, useRunChoices } from './RunMenu';
import { useStartRun } from './useStartRun';

/** Runs any branch, local or on origin; the server creates its worktree when it has none. */
export function RunBranchForm({ project, onSelectRun }: { project: Project; onSelectRun: (id: string) => void }) {
  const branches = useQuery({ queryKey: ['branches', project.id], queryFn: () => api.listBranches(project.id) });
  const { choices } = useRunChoices(project.id, null);
  const [branch, setBranch] = useState('');
  const [what, setWhat] = useState('');
  const start = useStartRun(project.id, onSelectRun);
  if (!choices.length) return null;

  return (
    <form
      className="flex flex-wrap items-center gap-1 px-2 py-1"
      onSubmit={(e) => {
        e.preventDefault();
        start.mutate({ branch, ...parseChoice(what) });
      }}
    >
      <select
        aria-label="Branch"
        value={branch}
        onChange={(e) => setBranch(e.target.value)}
        className="rounded-pdt bg-surface border border-border text-xs px-1 py-0.5"
      >
        <option value="">Branch…</option>
        {(branches.data ?? []).map((b) => (
          <option key={b.name} value={b.name}>{b.local ? b.name : `${b.name} (origin)`}</option>
        ))}
      </select>
      <RunMenu label="What to run" choices={choices} value={what} onChange={setWhat} />
      <Button type="submit" variant="ghost" aria-label="Run branch" disabled={!branch || !what || start.isPending}>Run</Button>
      {start.isError && <p role="alert" className="w-full text-xs text-failed">{start.error.message}</p>}
    </form>
  );
}
