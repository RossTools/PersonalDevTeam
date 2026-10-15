import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';

const field = 'w-full rounded-pdt border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function AddProjectForm() {
  const qc = useQueryClient();
  const [name, setName] = useState('');
  const [repoPath, setRepoPath] = useState('');
  const m = useMutation({
    mutationFn: api.createProject,
    onSuccess: () => {
      setName('');
      setRepoPath('');
      void qc.invalidateQueries({ queryKey: ['projects'] });
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    m.mutate({ name, repoPath });
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-2 p-2">
      <label className="text-xs text-muted">Name<input className={field} required aria-invalid={m.isError || undefined} value={name} onChange={(e) => setName(e.target.value)} /></label>
      <label className="text-xs text-muted">Repository path<input className={field} required aria-invalid={m.isError || undefined} value={repoPath} onChange={(e) => setRepoPath(e.target.value)} /></label>
      {m.isError && <p role="alert" className="text-xs text-failed">{m.error.message}</p>}
      <Button variant="primary" type="submit" disabled={m.isPending}>Add project</Button>
    </form>
  );
}
