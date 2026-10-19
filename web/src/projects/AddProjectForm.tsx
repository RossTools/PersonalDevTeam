import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useState, type FormEvent } from 'react';
import { Alert, Button, Field, Input, SegmentedControl } from '@ui';
import { api } from '../api/client';
import { FolderField } from '../common/FolderField';
import type { Project } from '../api/gen/Project';

type Mode = 'path' | 'clone';
const modes = [
  { value: 'path' as const, label: 'Folder' },
  { value: 'clone' as const, label: 'Clone URL' },
];

/** Adds a project from a folder on this host, or by cloning a repository URL into the Projects folder. */
export function AddProjectForm({ onAdded, autoFocus = false }: { onAdded?: (p: Project) => void; autoFocus?: boolean }) {
  const qc = useQueryClient();
  const errorId = useId();
  const [mode, setMode] = useState<Mode>('path');
  const [name, setName] = useState('');
  const [key, setKey] = useState('');
  const [repoPath, setRepoPath] = useState('');
  const [url, setUrl] = useState('');
  const [parentDir, setParentDir] = useState('');
  const host = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
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
  // A submit error moves focus to its Alert (spec §6.3).
  useEffect(() => {
    if (m.isError) document.getElementById(errorId)?.focus();
  }, [m.isError, m.error, errorId]);
  const opt = (v: string) => (v.trim() ? v.trim() : undefined);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (mode === 'path') add.mutate({ name, repoPath, ...(opt(key) ? { key: opt(key) } : {}) });
    else clone.mutate({ url, ...(opt(name) ? { name: opt(name) } : {}), ...(opt(key) ? { key: opt(key) } : {}), ...(opt(parentDir) ? { parentDir: opt(parentDir) } : {}) });
  };
  return (
    <form onSubmit={submit} aria-label="Add project" className="flex flex-col gap-3 p-2">
      <SegmentedControl aria-label="Add a project" size="sm" options={modes} value={mode} onChange={setMode} className="w-full [&>*]:flex-1" />
      {mode === 'path' ? (
        <>
          <Field label="Name">
            <Input required autoFocus={autoFocus} value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Repository path">
            <FolderField
              required
              title="Choose the repository"
              start={host.data?.projectsDir ?? ''}
              value={repoPath}
              onChange={setRepoPath}
              onPick={(f) => { if (!name.trim()) setName(f.name); }}
            />
          </Field>
        </>
      ) : (
        <>
          <Field label="Repository URL">
            <Input required autoFocus={autoFocus} mono placeholder="https://github.com/owner/repo" value={url} onChange={(e) => setUrl(e.target.value)} />
          </Field>
          <Field label="Into folder">
            <FolderField
              title="Clone into"
              placeholder={host.data?.projectsDir ?? '~/work'}
              start={host.data?.projectsDir ?? ''}
              value={parentDir}
              onChange={setParentDir}
            />
          </Field>
          <Field label="Name">
            <Input placeholder="the repository's name" value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
        </>
      )}
      <Field label="Key" hint="2–10 letters and digits">
        <Input mono className="uppercase placeholder:normal-case placeholder:font-sans" maxLength={10} placeholder="suggested from the name" value={key} onChange={(e) => setKey(e.target.value)} />
      </Field>
      {m.isError && <Alert id={errorId} tabIndex={-1} tone="error">{m.error.message}</Alert>}
      <Button variant="primary" type="submit" pending={m.isPending}>
        {mode === 'path' ? 'Add project' : clone.isPending ? 'Cloning…' : 'Clone and add'}
      </Button>
    </form>
  );
}
