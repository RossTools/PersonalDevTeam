import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useState, type FormEvent } from 'react';
import { Alert, Button, Field, Input, SegmentedControl } from '@ui';
import { api } from '../api/client';
import { FolderField } from '../common/FolderField';
import type { Project } from '../api/gen/Project';
import { invalidateProjects } from './queries';
import { useHostSettings } from '../host/queries';

export type AddProjectMode = 'path' | 'clone' | 'personal';
type Mode = AddProjectMode;
const modes = [
  { value: 'path' as const, label: 'Folder' },
  { value: 'clone' as const, label: 'Clone URL' },
  { value: 'personal' as const, label: 'Personal (no repo)' },
];

/**
 * Adds a project from a folder on this host, by cloning a repository URL into the Projects folder, or as a Personal
 * project with no repository (PDT makes its folder).
 */
export function AddProjectForm({ onAdded, autoFocus = false, initialMode = 'path' }: { onAdded?: (p: Project) => void; autoFocus?: boolean; initialMode?: Mode }) {
  const qc = useQueryClient();
  const errorId = useId();
  const [mode, setMode] = useState<Mode>(initialMode);
  const [name, setName] = useState('');
  const [key, setKey] = useState('');
  const [repoPath, setRepoPath] = useState('');
  const [url, setUrl] = useState('');
  const [parentDir, setParentDir] = useState('');
  const host = useHostSettings();
  const done = (p: Project) => {
    setName('');
    setKey('');
    setRepoPath('');
    setUrl('');
    void invalidateProjects(qc);
    onAdded?.(p);
  };
  const add = useMutation({ meta: { errorShown: true }, mutationFn: api.createProject, onSuccess: done });
  const clone = useMutation({ meta: { errorShown: true }, mutationFn: api.cloneProject, onSuccess: done });
  const m = mode === 'clone' ? clone : add;
  // A submit error moves focus to its Alert.
  useEffect(() => {
    if (m.isError) document.getElementById(errorId)?.focus();
  }, [m.isError, m.error, errorId]);
  const opt = (v: string) => (v.trim() ? v.trim() : undefined);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (mode === 'path') add.mutate({ name, repoPath, ...(opt(key) ? { key: opt(key) } : {}) });
    else if (mode === 'personal') add.mutate({ name, repoPath: '', kind: 'personal', ...(opt(key) ? { key: opt(key) } : {}) });
    else clone.mutate({ url, ...(opt(name) ? { name: opt(name) } : {}), ...(opt(key) ? { key: opt(key) } : {}), ...(opt(parentDir) ? { parentDir: opt(parentDir) } : {}) });
  };
  return (
    <form onSubmit={submit} aria-label="Add project" className="flex flex-col gap-3 p-2">
      <SegmentedControl aria-label="Add a project" size="sm" options={modes} value={mode} onChange={setMode} className="w-full [&>*]:flex-1" />
      {mode === 'path' ? (
        <>
          {/* The folder first: picking it fills the name in. */}
          <Field label="Repository path">
            <FolderField
              required
              autoFocus={autoFocus}
              title="Choose the repository"
              start={host.data?.projectsDir ?? ''}
              value={repoPath}
              onChange={setRepoPath}
              onPick={(f) => { if (!name.trim()) setName(f.name); }}
            />
          </Field>
          <Field label="Name">
            <Input required value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
        </>
      ) : mode === 'personal' ? (
        <>
          <Field label="Name" hint="No repository: PDT makes a folder for it, and each ticket works in its own folder there.">
            <Input required autoFocus={autoFocus} value={name} onChange={(e) => setName(e.target.value)} />
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
      <Field label="Key" hint="The prefix of its ticket numbers, like REPO in REPO-1: 2–10 letters and digits.">
        <Input mono className="uppercase placeholder:normal-case placeholder:font-sans" maxLength={10} placeholder="suggested from the name" value={key} onChange={(e) => setKey(e.target.value)} />
      </Field>
      {m.isError && <Alert id={errorId} tabIndex={-1} tone="error">{m.error.message}</Alert>}
      <Button variant="primary" type="submit" pending={m.isPending}>
        {mode !== 'clone' ? 'Add project' : clone.isPending ? 'Cloning…' : 'Clone and add'}
      </Button>
    </form>
  );
}
