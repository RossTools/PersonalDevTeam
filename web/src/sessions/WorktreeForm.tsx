import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';

const field = 'w-full rounded-pdt border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function WorktreeForm({ projectId }: { projectId: string }) {
  const qc = useQueryClient();
  const [branch, setBranch] = useState('');
  const m = useMutation({
    mutationFn: (b: string) => api.createWorktree(projectId, { branch: b }),
    onSuccess: () => {
      setBranch('');
      void qc.invalidateQueries({ queryKey: ['worktrees', projectId] });
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    m.mutate(branch);
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-2 px-2">
      <label className="text-xs text-muted">
        New worktree branch
        <input className={field} value={branch} required aria-invalid={m.isError || undefined} onChange={(e) => setBranch(e.target.value)} />
      </label>
      {m.isError && <p role="alert" className="text-xs text-failed">{m.error.message}</p>}
      <Button type="submit" disabled={m.isPending}>Create worktree</Button>
    </form>
  );
}
