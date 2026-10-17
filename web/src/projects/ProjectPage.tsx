import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ExternalLink } from 'lucide-react';
import { Button } from '@ui';
import { api, ApiError } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { Worktree } from '../api/gen/Worktree';
import { ProjectSettingsPanel } from '../runs/ProjectSettingsPanel';
import { branchUrl, compareUrl, repoUrl } from './github';

const h = 'text-xs uppercase tracking-wider text-muted';
const field = 'rounded-pdt border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';
const link = 'inline-flex items-center gap-1 text-xs text-accent underline';

function Ext({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" className={link}>
      {children}
      <ExternalLink size={11} aria-hidden />
    </a>
  );
}

function Identity({ project }: { project: Project }) {
  const qc = useQueryClient();
  const [name, setName] = useState(project.name);
  const [key, setKey] = useState(project.key);
  const save = useMutation({
    mutationFn: () => api.updateProject(project.id, {
      ...(name.trim() !== project.name ? { name } : {}),
      ...(key.trim().toUpperCase() !== project.key ? { key } : {}),
    }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['projects'] });
      void qc.invalidateQueries({ queryKey: ['workspaces'] });
    },
  });
  const changed = name.trim() !== project.name || key.trim().toUpperCase() !== project.key;
  return (
    <section aria-label="Project" className="flex flex-col gap-2">
      <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => { e.preventDefault(); if (changed) save.mutate(); }}>
        <label className="flex flex-col gap-1 text-xs text-muted">Name
          <input aria-label="Project name" className={`${field} w-64`} value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="flex flex-col gap-1 text-xs text-muted">Key
          <input aria-label="Project key" className={`${field} w-24 uppercase placeholder:normal-case font-mono`} maxLength={10} value={key} onChange={(e) => setKey(e.target.value)} />
        </label>
        <Button type="submit" variant="secondary" disabled={!changed || save.isPending}>Save name and key</Button>
      </form>
      <p className="text-xs text-muted">The key is the project's short code (2–10 letters and digits); tickets will be numbered with it, like {project.key}-42.</p>
      {save.isError && <p role="alert" className="text-xs text-failed">{save.error.message}</p>}
      <p className="text-xs text-muted flex flex-wrap items-center gap-3">
        <span className="font-mono" title="Repository folder">{project.repoPath}</span>
        {project.githubRepo && <Ext href={repoUrl(project.githubRepo)}>{project.githubRepo} on GitHub</Ext>}
      </p>
    </section>
  );
}

function WorktreeRow({ project, wt }: { project: Project; wt: Worktree }) {
  const qc = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [dirty, setDirty] = useState(false);
  const remove = useMutation({
    mutationFn: (force: boolean) => api.removeWorktree(wt.id, { force, deleteBranch }),
    onError: (e) => { if (e instanceof ApiError && e.code === 'worktree_dirty') setDirty(true); },
    onSettled: (_d, e) => {
      if (e instanceof ApiError && e.code === 'worktree_dirty') return;
      void qc.invalidateQueries({ queryKey: ['worktrees', project.id] });
      void qc.invalidateQueries({ queryKey: ['sessions', project.id] });
      void qc.invalidateQueries({ queryKey: ['workspaces'] });
    },
  });
  const error = remove.error && !(remove.error instanceof ApiError && remove.error.code === 'worktree_dirty') ? remove.error : null;
  return (
    <li className="flex flex-col gap-1 rounded-pdt border border-border p-2">
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-medium">{wt.branch}</span>
        {wt.missing && <span className="text-[11px] text-waiting">folder missing</span>}
        {project.githubRepo && !wt.missing && (
          <>
            <Ext href={branchUrl(project.githubRepo, wt.branch)}>Branch</Ext>
            <Ext href={compareUrl(project.githubRepo, project.baseBranch, wt.branch)}>Open PR</Ext>
          </>
        )}
        <span className="flex-1" />
        {wt.missing ? (
          <Button variant="ghost" disabled={remove.isPending} onClick={() => remove.mutate(false)}>Forget</Button>
        ) : !confirming ? (
          <Button variant="ghost" onClick={() => setConfirming(true)}>Remove…</Button>
        ) : null}
      </div>
      <span className="text-[11px] text-muted font-mono truncate" title={wt.path}>{wt.path}</span>
      {confirming && !wt.missing && (
        <div role="group" aria-label={`Remove ${wt.branch}`} className="flex flex-wrap items-center gap-2 text-xs">
          {dirty ? (
            <span className="text-waiting">This worktree has uncommitted changes. Removing it discards them.</span>
          ) : (
            <span>Remove the worktree folder? Its ended sessions are forgotten.</span>
          )}
          <label className="flex items-center gap-1">
            <input type="checkbox" checked={deleteBranch} onChange={(e) => setDeleteBranch(e.target.checked)} />
            Also delete the branch
          </label>
          <Button variant="secondary" className={dirty ? 'text-failed' : ''} disabled={remove.isPending} onClick={() => remove.mutate(dirty)}>
            {dirty ? 'Remove anyway' : 'Remove'}
          </Button>
          <Button variant="ghost" onClick={() => { setConfirming(false); setDirty(false); }}>Cancel</Button>
        </div>
      )}
      {error && <p role="alert" className="text-xs text-failed">{error.message}</p>}
    </li>
  );
}

function Worktrees({ project }: { project: Project }) {
  const list = useQuery({ queryKey: ['worktrees', project.id], queryFn: () => api.listWorktrees(project.id) });
  return (
    <section aria-label="Worktrees" className="flex flex-col gap-2">
      <h3 className={h}>Worktrees</h3>
      {list.data?.length === 0 && <p className="text-xs text-muted">No worktrees. Running a branch makes one.</p>}
      <ul className="flex flex-col gap-1">
        {list.data?.map((w) => <WorktreeRow key={w.id} project={project} wt={w} />)}
      </ul>
    </section>
  );
}

function RemoveProject({ project, onRemoved }: { project: Project; onRemoved: () => void }) {
  const qc = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const remove = useMutation({
    mutationFn: () => api.deleteProject(project.id),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['projects'] });
      void qc.invalidateQueries({ queryKey: ['workspaces'] });
      onRemoved();
    },
  });
  return (
    <section aria-label="Remove project" className="flex flex-col gap-2 border-t border-border pt-3">
      {confirming ? (
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span>Remove {project.name} from PDT? Its sessions and runs stop. Files on disk stay.</span>
          <Button variant="secondary" className="text-failed" disabled={remove.isPending} onClick={() => remove.mutate()}>Remove project</Button>
          <Button variant="ghost" onClick={() => setConfirming(false)}>Cancel</Button>
        </div>
      ) : (
        <Button variant="ghost" className="self-start text-failed" onClick={() => setConfirming(true)}>Remove project…</Button>
      )}
      {remove.isError && <p role="alert" className="text-xs text-failed">{remove.error.message}</p>}
    </section>
  );
}

/** A project's page in Settings: identity, worktrees, run settings on this host, removal. */
export function ProjectPage({ project, onRemoved }: { project: Project; onRemoved: () => void }) {
  return (
    <div className="flex flex-col gap-4 p-4 text-sm overflow-auto">
      <Identity key={`${project.name}:${project.key}`} project={project} />
      <Worktrees project={project} />
      <div className="-mx-4 -mt-2 border-t border-border"><ProjectSettingsPanel project={project} /></div>
      <RemoveProject project={project} onRemoved={onRemoved} />
    </div>
  );
}
