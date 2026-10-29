import { useId } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Trash2 } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, Link, Section, Select, SettingsRow, Skeleton, useConfirm } from '@ui';
import { AutoSaveText, SaveStatus, useSaveState } from '../common/AutoSave';
import { api, ApiError } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { Worktree } from '../api/gen/Worktree';
import { BranchChip, ProjectKey } from '../common/BranchChip';
import { branchOptions } from '../common/branchOptions';
import { useAction } from '../common/useAction';
import { useProjectSettingsSections } from '../runs/ProjectSettingsPanel';
import { SettingsScreens, type SettingsSection } from '../common/SettingsScreens';
import { repoUrl } from './forgeLinks';
import { useOpenChanges } from '../review/open';

/**
 * Name, key and base branch, each saved as it changes (PDT-231): text on blur or Enter, the branch at once. Folder and
 * GitHub link below. A Personal project has no repository, so no base branch either.
 */
function General({ project, onKeyChange }: { project: Project; onKeyChange?: (key: string) => void }) {
  const personal = project.kind === 'personal';
  const qc = useQueryClient();
  const branches = useQuery({ queryKey: ['branches', project.id], queryFn: () => api.listBranches(project.id), enabled: !personal });
  const baseOptions = branchOptions(branches.data ?? [], project.baseBranch);
  const [baseState, trackBase] = useSaveState();
  const save = async (p: { name?: string; key?: string; baseBranch?: string }) => {
    const saved = await api.updateProject(project.id, p);
    await qc.invalidateQueries({ queryKey: ['projects'] });
    void qc.invalidateQueries({ queryKey: ['workspaces'] });
    // The page's path names the project by its key.
    if (saved.key !== project.key) onKeyChange?.(saved.key);
    return saved;
  };
  const nameId = useId();
  const keyId = useId();
  const baseId = useId();
  return (
    <Section label="General" description="The project's name, key and the branch its work starts from.">
      <SettingsRow label="Name" htmlFor={nameId}>
        <AutoSaveText id={nameId} aria-label="Project name" className="w-64 max-w-full" value={project.name} onSave={(name) => save({ name })} />
      </SettingsRow>
      <SettingsRow
        label="Key"
        htmlFor={keyId}
        description={`The project's short code (2–10 letters and digits); tickets are numbered with it, like ${project.key}-42.`}
      >
        <AutoSaveText id={keyId} aria-label="Project key" mono className="w-28 uppercase" maxLength={10} value={project.key}
          onSave={(key) => save({ key })} />
      </SettingsRow>
      {!personal && (
        <SettingsRow
          label="Base branch"
          htmlFor={baseId}
          description="New worktrees and tickets branch from it, and their changes are reviewed and merged against it."
        >
          <SaveStatus state={baseState} />
          <Select id={baseId} aria-label="Base branch" className="w-64 max-w-full" searchable options={baseOptions} value={project.baseBranch}
            onChange={(baseBranch) => void trackBase(save({ baseBranch }))} />
        </SettingsRow>
      )}
      {branches.isError && <Alert tone="error">Branches could not be loaded: {branches.error.message}</Alert>}
      {personal ? (
        <SettingsRow label="Folder" description="Personal project (no repository): each ticket works in its own folder here.">
          <span className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={project.repoPath}>{project.repoPath}</span>
        </SettingsRow>
      ) : (
        <SettingsRow label="Repository folder">
          <span className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={project.repoPath}>{project.repoPath}</span>
        </SettingsRow>
      )}
      {project.forge && (
        <SettingsRow label={project.forge.name}>
          <Link external href={repoUrl(project.forge)} className="text-xs">{project.forge.repo} on {project.forge.name}</Link>
        </SettingsRow>
      )}
    </Section>
  );
}

function WorktreeRow({ project, wt }: { project: Project; wt: Worktree }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const openChanges = useOpenChanges();
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
    <li className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <BranchChip
          branch={wt.branch}
          forge={wt.missing ? null : project.forge ?? null}
          base={project.baseBranch}
          {...(wt.missing ? {} : { onReview: () => openChanges({ id: wt.id, projectId: project.id }) })}
        />
        {wt.missing && <Badge tone="waiting">folder missing</Badge>}
        <span className="flex-1" />
        {!wt.missing && <Button size="sm" variant="ghost" onClick={() => openChanges({ id: wt.id, projectId: project.id })}>Review changes</Button>}
        {wt.missing ? (
          <Button size="sm" pending={forgetting} onClick={() => void forget()}>Forget</Button>
        ) : (
          <Button size="sm" pending={removing} onClick={() => void remove()}><Trash2 aria-hidden /> Remove…</Button>
        )}
      </div>
      <span className="truncate font-mono text-meta text-muted-foreground" title={wt.path}>{wt.path}</span>
    </li>
  );
}

function Worktrees({ project }: { project: Project }) {
  const list = useQuery({ queryKey: ['worktrees', project.id], queryFn: () => api.listWorktrees(project.id) });
  return (
    <Section label="Worktrees" description="Checkouts in .worktrees, one per branch. Removing one deletes its folder.">
      {list.isPending && <Skeleton lines={3} label="Loading worktrees" />}
      {list.isError && <Alert tone="error">Worktrees could not be loaded: {list.error.message}</Alert>}
      {list.data?.length === 0 && <EmptyState>No worktrees. Running a branch makes one.</EmptyState>}
      {!!list.data?.length && (
        <ul data-flush className="flex flex-col divide-y divide-border">
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
      typeToConfirm: project.name,
    });
    if (!ok) return;
    await api.deleteProject(project.id);
    void qc.invalidateQueries({ queryKey: ['projects'] });
    void qc.invalidateQueries({ queryKey: ['workspaces'] });
    onRemoved();
  });
  return (
    <Section label="Danger zone" danger>
      <SettingsRow label="Remove this project" description="PDT forgets it and stops its sessions and runs. Files on disk stay.">
        <Button size="sm" variant="danger" pending={removing} onClick={() => void remove()}>Remove project…</Button>
      </SettingsRow>
    </Section>
  );
}

export interface ProjectPageProps {
  project: Project;
  onRemoved: () => void;
  /** A phone: one screen per section, the list of them while `section` is null. */
  drill?: boolean;
  section?: string | null;
  onSection?: (slug: string) => void;
  /** The project's key changed: its page's path changes with it. */
  onKeyChange?: (key: string) => void;
}

/**
 * A project's page in Settings: identity, worktrees, run settings on this host, removal. A Personal project (no
 * repository) has no worktrees, runs or merging.
 */
export function ProjectPage({ project, onRemoved, drill = false, section = null, onSection, onKeyChange }: ProjectPageProps) {
  const panel = useProjectSettingsSections(project);
  const sections: SettingsSection[] = [
    { label: 'General', node: <General project={project} onKeyChange={onKeyChange} /> },
    ...(project.kind !== 'personal' ? [{ label: 'Worktrees', node: <Worktrees project={project} /> }] : []),
    ...panel.sections,
    { label: 'Danger zone', node: <RemoveProject project={project} onRemoved={onRemoved} /> },
  ];
  return (
    <SettingsScreens
      title={project.name}
      actions={<ProjectKey projectKey={project.key} />}
      crumb={<span className="flex items-center gap-1.5">Projects › {project.name} <ProjectKey projectKey={project.key} /></span>}
      notice={panel.notice}
      sections={sections}
      drill={drill}
      section={section}
      onSection={onSection}
    />
  );
}
