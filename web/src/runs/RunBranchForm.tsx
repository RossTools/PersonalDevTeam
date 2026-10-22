import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Alert, Button, Select } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { branchOptions } from '../common/branchOptions';
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
      <Select
        aria-label="Branch"
        size="sm"
        placeholder="Branch…"
        searchable
        options={branchOptions(branches.data ?? [])}
        value={branch}
        onChange={setBranch}
      />
      <RunMenu label="What to run" choices={choices} value={what} onChange={setWhat} />
      <Button type="submit" variant="ghost" size="sm" aria-label="Run branch" disabled={!branch || !what} pending={start.isPending}>
        Run
      </Button>
      {branches.isError && <Alert tone="error" className="w-full">{branches.error.message}</Alert>}
      {start.isError && <Alert tone="error" className="w-full">{start.error.message}</Alert>}
    </form>
  );
}
