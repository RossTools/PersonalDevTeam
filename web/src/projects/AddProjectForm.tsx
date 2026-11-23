import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { FolderOpen, Link2, NotebookPen, Network } from 'lucide-react';
import { Alert, Button, ChoiceCardGroup, Field, Input, Spinner } from '@ui';
import { api } from '../api/client';
import { ApiError } from '../api/core';
import { FolderField } from '../common/FolderField';
import type { CloneJob } from '../api/gen/CloneJob';
import type { Project } from '../api/gen/Project';
import { invalidateProjects, projectKeys, useClones } from './queries';
import { useHostSettings } from '../host/queries';
import { JoinRemoteProjectForm } from './JoinRemoteProjectForm';

export type AddProjectMode = 'path' | 'clone' | 'personal' | 'remote';
type Mode = AddProjectMode;
/** A mode's card: an icon before its title, and the title as the radio's name. */
const card = (value: Mode, icon: ReactNode, title: string, description: string) => ({
  value,
  'aria-label': title,
  title: <span className="flex items-center gap-1.5 [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-muted-foreground">{icon}{title}</span>,
  description,
});
const modes = [
  card('path', <FolderOpen aria-hidden />, 'Folder', 'A repo on this host'),
  card('clone', <Link2 aria-hidden />, 'Clone a URL', 'Clone into Projects'),
  card('personal', <NotebookPen aria-hidden />, 'Personal', 'No repository'),
  card('remote', <Network aria-hidden />, 'Join a remote', 'Another PDT owns it'),
];

/**
 * Adds a project from a folder on this host, by cloning a repository URL into the Projects folder, or as a Personal
 * project with no repository (PDT makes its folder); or joins a remote project of another instance (`onJoined`;
 * `onLeave` runs before it goes to turn remote access on).
 */
export function AddProjectForm({ onAdded, onJoined, onLeave, autoFocus = false, initialMode = 'path' }: {
  onAdded?: (p: Project) => void;
  onJoined?: () => void;
  onLeave?: () => void;
  autoFocus?: boolean;
  initialMode?: Mode;
}) {
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
  // The clones under way go on after a reload, so they're shown whoever started them.
  const clones = useClones({ enabled: mode === 'clone' });
  const cancel = useMutation({ mutationFn: api.cancelClone, onSettled: () => qc.invalidateQueries({ queryKey: projectKeys.clones }) });
  // A clone that left the list has finished (or failed): one started before a reload adds its project unseen.
  const seen = useRef<string[]>([]);
  useEffect(() => {
    if (!clones.data) return;
    const now = clones.data.map((c) => c.id);
    if (seen.current.some((id) => !now.includes(id))) void invalidateProjects(qc);
    seen.current = now;
  }, [clones.data, qc]);
  // A clone the user cancelled needs no error.
  const failed = m.isError && !(m.error instanceof ApiError && m.error.code === 'clone_cancelled');
  // A submit error moves focus to its Alert.
  useEffect(() => {
    if (failed) document.getElementById(errorId)?.focus();
  }, [failed, m.error, errorId]);
  const opt = (v: string) => (v.trim() ? v.trim() : undefined);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (mode === 'path') add.mutate({ name, repoPath, ...(opt(key) ? { key: opt(key) } : {}) });
    else if (mode === 'personal') add.mutate({ name, repoPath: '', kind: 'personal', ...(opt(key) ? { key: opt(key) } : {}) });
    else clone.mutate({ url, ...(opt(name) ? { name: opt(name) } : {}), ...(opt(key) ? { key: opt(key) } : {}), ...(opt(parentDir) ? { parentDir: opt(parentDir) } : {}) });
  };
  // Two columns on phones too: four cards one under another push the form off the screen.
  const choose = <ChoiceCardGroup aria-label="Add a project" size="sm" options={modes} value={mode} onChange={setMode} className="grid-cols-2 sm:grid-cols-2" />;
  if (mode === 'remote') {
    return (
      <div className="flex flex-col gap-3 p-2">
        {choose}
        <JoinRemoteProjectForm autoFocus={autoFocus} onJoined={() => onJoined?.()} onLeave={onLeave} />
      </div>
    );
  }
  return (
    <form onSubmit={submit} aria-label="Add project" className="flex flex-col gap-3 p-2">
      {choose}
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
      {mode === 'clone' && clones.data?.map((c) => <CloneProgress key={c.id} job={c} onCancel={() => cancel.mutate(c.id)} />)}
      {failed && <Alert id={errorId} tabIndex={-1} tone="error">{m.error?.message}</Alert>}
      <Button variant="primary" type="submit" pending={m.isPending}>
        {mode !== 'clone' ? 'Add project' : clone.isPending ? 'Cloning…' : 'Clone and add'}
      </Button>
    </form>
  );
}

/** A clone under way: where it goes, git's latest progress, and Cancel (which removes what it cloned so far). */
function CloneProgress({ job, onCancel }: { job: CloneJob; onCancel: () => void }) {
  return (
    <div role="group" aria-label={`Cloning ${job.name}`} className="flex items-center gap-2 rounded-md border p-2 text-sm">
      <Spinner decorative className="size-4 shrink-0 text-muted-foreground" />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate">Cloning into <span className="font-mono">{job.dest}</span></span>
        <span className="truncate font-mono text-xs text-muted-foreground" aria-live="polite">{job.progress || 'Starting…'}</span>
      </div>
      <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
    </div>
  );
}
