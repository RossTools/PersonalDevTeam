import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';

const field = 'w-full rounded-pdt border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';
const tab = (on: boolean) => `flex-1 rounded-pdt px-2 py-0.5 text-xs ${on ? 'bg-accent text-fg' : 'text-muted-foreground hover:bg-surface-2'}`;

/** Adds a project from a folder on this host, or by cloning a repository URL into the Projects folder. */
export function AddProjectForm({ onAdded }: { onAdded?: (p: Project) => void }) {
  const qc = useQueryClient();
  const [mode, setMode] = useState<'path' | 'clone'>('path');
  const [name, setName] = useState('');
  const [key, setKey] = useState('');
  const [repoPath, setRepoPath] = useState('');
  const [url, setUrl] = useState('');
  const [parentDir, setParentDir] = useState('');
  const host = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings, enabled: mode === 'clone' });
  const done = (p: Project) => {
    setName('');
    setKey('');
    setRepoPath('');
    setUrl('');
    void qc.invalidateQueries({ queryKey: ['projects'] });
    void qc.invalidateQueries({ queryKey: ['workspaces'] });
    onAdded?.(p);
  };
  const add = useMutation({ mutationFn: api.createProject, onSuccess: done });
  const clone = useMutation({ mutationFn: api.cloneProject, onSuccess: done });
  const m = mode === 'path' ? add : clone;
  const opt = (v: string) => (v.trim() ? v.trim() : undefined);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (mode === 'path') add.mutate({ name, repoPath, ...(opt(key) ? { key: opt(key) } : {}) });
    else clone.mutate({ url, ...(opt(name) ? { name: opt(name) } : {}), ...(opt(key) ? { key: opt(key) } : {}), ...(opt(parentDir) ? { parentDir: opt(parentDir) } : {}) });
  };
  return (
    <form onSubmit={submit} aria-label="Add project" className="flex flex-col gap-2 p-2">
      <div role="tablist" aria-label="Add a project" className="flex gap-1">
        <button type="button" role="tab" aria-selected={mode === 'path'} className={tab(mode === 'path')} onClick={() => setMode('path')}>Folder</button>
        <button type="button" role="tab" aria-selected={mode === 'clone'} className={tab(mode === 'clone')} onClick={() => setMode('clone')}>Clone URL</button>
      </div>
      {mode === 'path' ? (
        <>
          <label className="text-xs text-muted-foreground">Name<input className={field} required value={name} onChange={(e) => setName(e.target.value)} /></label>
          <label className="text-xs text-muted-foreground">Repository path<input className={field} required value={repoPath} onChange={(e) => setRepoPath(e.target.value)} /></label>
        </>
      ) : (
        <>
          <label className="text-xs text-muted-foreground">Repository URL<input className={field} required placeholder="https://github.com/owner/repo" value={url} onChange={(e) => setUrl(e.target.value)} /></label>
          <label className="text-xs text-muted-foreground">Into folder<input className={field} placeholder={host.data?.projectsDir ?? '~/work'} value={parentDir} onChange={(e) => setParentDir(e.target.value)} /></label>
          <label className="text-xs text-muted-foreground">Name<input className={field} placeholder="the repository's name" value={name} onChange={(e) => setName(e.target.value)} /></label>
        </>
      )}
      <label className="text-xs text-muted-foreground">Key<input className={`${field} uppercase placeholder:normal-case font-mono`} maxLength={10} placeholder="suggested from the name" value={key} onChange={(e) => setKey(e.target.value)} /></label>
      {m.isError && <p role="alert" className="text-xs text-failed">{m.error.message}</p>}
      <Button variant="primary" type="submit" disabled={m.isPending}>
        {mode === 'path' ? 'Add project' : clone.isPending ? 'Cloning…' : 'Clone and add'}
      </Button>
    </form>
  );
}
