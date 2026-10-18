import { useEffect, useId, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, EmptyState, Field, Input, Link, Page, Section, SettingsRow, Skeleton, useConfirm } from '@ui';
import { api, ApiError } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { Worktree } from '../api/gen/Worktree';
import { BranchChip, ProjectKey } from '../common/BranchChip';
import { useAction } from '../common/useAction';
import { ProjectSettingsPanel } from '../runs/ProjectSettingsPanel';
import { repoUrl } from './github';

/** Name and key: one form, one Save (spec §6.1). Folder and GitHub link below. */
function Identity({ project }: { project: Project }) {
  const qc = useQueryClient();
  const errorId = useId();
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
  useEffect(() => {
    if (save.isError) document.getElementById(errorId)?.focus();
  }, [save.isError, save.error, errorId]);
  const changed = name.trim() !== project.name || key.trim().toUpperCase() !== project.key;
  return (
    <form onSubmit={(e) => { e.preventDefault(); if (changed) save.mutate(); }}>
      <Section
        label="Project"
        actions={<Button type="submit" size="sm" variant="primary" disabled={!changed} pending={save.isPending}>Save</Button>}
      >
        <div className="flex flex-wrap items-start gap-3">
          <Field label="Project name" className="w-64 max-w-full">
            <Input value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Project key" className="w-28">
            <Input mono className="uppercase" maxLength={10} value={key} onChange={(e) => setKey(e.target.value)} />
          </Field>
        </div>
        <p className="text-xs text-muted-foreground">
          The key is the project's short code (2–10 letters and digits); tickets will be numbered with it, like {project.key}-42.
        </p>
        {save.isError && <Alert id={errorId} tabIndex={-1} tone="error">{save.error.message}</Alert>}
        <SettingsRow label="Repository folder">
          <span className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={project.repoPath}>{project.repoPath}</span>
        </SettingsRow>
        {project.githubRepo && (
          <SettingsRow label="GitHub">
            <Link external href={repoUrl(project.githubRepo)} className="text-xs">{project.githubRepo} on GitHub</Link>
          </SettingsRow>
        )}
      </Section>
    </form>
  );
}

function WorktreeRow({ project, wt }: { project: Project; wt: Worktree }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const settle = () => {
    void qc.invalidateQueries({ queryKey: ['worktrees', project.id] });
    void qc.invalidateQueries({ queryKey: ['sessions', project.id] });
    void qc.invalidateQueries({ queryKey: ['workspaces'] });
  };
  const [forget, forgetting] = useAction(async () => {
    try {
      await api.removeWorktree(wt.id, { force: false, deleteBranch: false });
    } finally {
      settle();
    }
  });
  const [remove, removing] = useAction(async () => {
    const first = await confirm({
      title: `Remove worktree ${wt.branch}?`,
      body: 'The worktree folder is deleted and its ended sessions are forgotten.',
      action: 'Remove worktree',
      danger: true,
      checkbox: { label: 'Also delete the branch' },
    });
    if (!first.ok) return;
    try {
      await api.removeWorktree(wt.id, { force: false, deleteBranch: first.checked });
    } catch (e) {
      if (!(e instanceof ApiError && e.code === 'worktree_dirty')) {
        settle();
        throw e;
      }
      // Uncommitted changes: ask again, saying what is lost.
      const again = await confirm({
        title: `${wt.branch} has uncommitted changes`,
        body: 'This worktree has uncommitted changes. Removing it discards them.',
        action: 'Remove anyway',
        danger: true,
        checkbox: { label: 'Also delete the branch', defaultChecked: first.checked },
      });
      if (!again.ok) return;
      try {
        await api.removeWorktree(wt.id, { force: true, deleteBranch: again.checked });
      } finally {
        settle();
      }
      return;
    }
    settle();
  });
  return (
    <li className="flex flex-col gap-1 rounded-md border border-border p-2">
      <div className="flex flex-wrap items-center gap-2">
        <BranchChip branch={wt.branch} repo={wt.missing ? null : project.githubRepo} base={project.baseBranch} />
        {wt.missing && <Badge tone="waiting">folder missing</Badge>}
        <span className="flex-1" />
        {wt.missing ? (
          <Button size="sm" variant="ghost" pending={forgetting} onClick={() => void forget()}>Forget</Button>
        ) : (
          <Button size="sm" variant="ghost" pending={removing} onClick={() => void remove()}>Remove…</Button>
        )}
      </div>
      <span className="truncate font-mono text-meta text-muted-foreground" title={wt.path}>{wt.path}</span>
    </li>
  );
}

function Worktrees({ project }: { project: Project }) {
  const list = useQuery({ queryKey: ['worktrees', project.id], queryFn: () => api.listWorktrees(project.id) });
  return (
    <Section label="Worktrees">
      {list.isPending && <Skeleton lines={3} label="Loading worktrees" />}
      {list.isError && <Alert tone="error">Worktrees could not be loaded: {list.error.message}</Alert>}
      {list.data?.length === 0 && <EmptyState>No worktrees. Running a branch makes one.</EmptyState>}
      {!!list.data?.length && (
        <ul className="flex flex-col gap-1">
          {list.data.map((w) => <WorktreeRow key={w.id} project={project} wt={w} />)}
        </ul>
      )}
    </Section>
  );
}

function RemoveProject({ project, onRemoved }: { project: Project; onRemoved: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [remove, removing] = useAction(async () => {
    const { ok } = await confirm({
      title: `Remove ${project.name}?`,
      body: 'PDT forgets the project; its sessions and runs stop. Files on disk stay.',
      action: 'Remove project',
      danger: true,
    });
    if (!ok) return;
    await api.deleteProject(project.id);
    void qc.invalidateQueries({ queryKey: ['projects'] });
    void qc.invalidateQueries({ queryKey: ['workspaces'] });
    onRemoved();
  });
  return (
    <Section label="Danger zone" danger>
      <SettingsRow label="Remove project" description="Stops its sessions and runs. Files on disk stay.">
        <Button size="sm" variant="danger" pending={removing} onClick={() => void remove()}>Remove project…</Button>
      </SettingsRow>
    </Section>
  );
}

/** A project's page in Settings: identity, worktrees, run settings on this host, removal. */
export function ProjectPage({ project, onRemoved }: { project: Project; onRemoved: () => void }) {
  return (
    <Page title={project.name} actions={<ProjectKey projectKey={project.key} />}>
      <Identity key={`${project.name}:${project.key}`} project={project} />
      <Worktrees project={project} />
      <ProjectSettingsPanel project={project} />
      <RemoveProject project={project} onRemoved={onRemoved} />
    </Page>
  );
}
