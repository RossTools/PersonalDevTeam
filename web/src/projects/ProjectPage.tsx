import { useId } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Trash2 } from 'lucide-react';
import { Alert, Badge, Button, EmptyState, Link, Section, Select, SettingsRow, Skeleton, useConfirm } from '@ui';
import { AutoSaveText, SaveStatus, useSaveState } from '../common/AutoSave';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { Worktree } from '../api/gen/Worktree';
import type { StorageSettings } from '../api/gen/StorageSettings';
import type { WontDoCleanup } from '../api/gen/WontDoCleanup';
import type { WorktreeStorage } from '../api/gen/WorktreeStorage';
import { TicketKey } from '../common/TicketKey';
import { CleanButton, sizeText, StageBadge, useRemoveWorktree, useStorage } from './worktreeStorage';
import { BranchChip, ProjectKey } from '../common/BranchChip';
import { branchOptions } from '../common/branchOptions';
import { useAction } from '../common/useAction';
import { useProjectSettingsSections } from '../runs/ProjectSettingsPanel';
import { SettingsScreens, type SettingsSection } from '../common/SettingsScreens';
import { repoUrl } from './forgeLinks';
import { invalidateWorktrees, worktreesQuery } from '../common/worktrees';
import { useOpenChanges } from '../review/open';
import { ProjectDocuments } from '../documents/DocumentSettings';
import { formatBytes } from '../common/text';
import { invalidateProjects, projectKeys } from './queries';

/**
 * Name, key and base branch, each saved as it changes (PDT-231): text on blur or Enter, the branch at once. Folder and
 * GitHub link below. A Personal project has no repository, so no base branch either.
 */
function General({ project, onKeyChange }: { project: Project; onKeyChange?: (key: string) => void }) {
  const personal = project.kind === 'personal';
  const qc = useQueryClient();
  const branches = useQuery({ queryKey: projectKeys.branches(project.id), queryFn: () => api.listBranches(project.id), enabled: !personal });
  const baseOptions = branchOptions(branches.data ?? [], project.baseBranch);
  const [baseState, trackBase] = useSaveState();
  const save = async (p: { name?: string; key?: string; baseBranch?: string }) => {
    const saved = await api.updateProject(project.id, p);
    await invalidateProjects(qc);
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

function WorktreeRow({ project, wt, storage }: { project: Project; wt: Worktree; storage: WorktreeStorage | null }) {
  const qc = useQueryClient();
  const openChanges = useOpenChanges();
  const size = storage?.size ?? null;
  const [forget, forgetting] = useAction(async () => {
    try {
      await api.removeWorktree(wt.id, { force: false, deleteBranch: false });
    } finally {
      invalidateWorktrees(qc, project.id);
      void qc.invalidateQueries({ queryKey: ['storage'] });
    }
  });
  const [remove, removing] = useRemoveWorktree(wt, size);
  return (
    <li className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <BranchChip
          branch={wt.branch}
          forge={wt.missing ? null : project.forge ?? null}
          base={project.baseBranch}
          {...(wt.missing ? {} : { onReview: () => openChanges({ id: wt.id, projectId: project.id }) })}
        />
        {storage?.ticket && <TicketKey ticketKey={storage.ticket.key} title={storage.ticket.title} />}
        {storage?.ticket && <StageBadge stage={storage.ticket.stage} />}
        {wt.missing && <Badge tone="waiting">folder missing</Badge>}
        <span className="flex-1" />
        {!wt.missing && <Button size="sm" variant="ghost" onClick={() => openChanges({ id: wt.id, projectId: project.id })}>Review changes</Button>}
        {!wt.missing && <CleanButton id={wt.id} label="Clean" />}
        {wt.missing ? (
          <Button size="sm" pending={forgetting} onClick={() => void forget()}>Forget</Button>
        ) : (
          <Button size="sm" pending={removing} onClick={() => void remove()}><Trash2 aria-hidden /> Remove…</Button>
        )}
      </div>
      <span className="flex flex-wrap gap-x-3 text-meta text-muted-foreground">
        <span className="min-w-0 truncate font-mono" title={wt.path}>{wt.path}</span>
        {!wt.missing && <span>{sizeText(size)}</span>}
      </span>
    </li>
  );
}

function days(text: string): number {
  if (!/^\d+$/.test(text) || Number(text) > 365) throw new Error('a whole number of days up to 365 (0: never)');
  return Number(text);
}

const WONT_DO_OPTIONS: { value: WontDoCleanup; label: string }[] = [
  { value: 'now', label: 'Clean up at once' },
  { value: 'later', label: 'Keep for some days' },
  { value: 'keep', label: 'Keep' },
];

/** The project's automatic cleanup of finished tickets' worktrees. */
function Cleanup({ project, settings }: { project: Project; settings: StorageSettings }) {
  const qc = useQueryClient();
  const save = async (s: Partial<StorageSettings>) => {
    await api.putProjectStorage(project.id, { ...settings, ...s });
    await qc.invalidateQueries({ queryKey: ['storage'] });
  };
  return (
    <>
      <SettingsRow label="Clean build output after" description="Days after a ticket is Done or Won't do that its worktree's build output is removed (0: never).">
        <AutoSaveText aria-label="Clean build output after (days)" inputMode="numeric" className="w-20" value={String(settings.cleanAfterDays)}
          onSave={(t) => save({ cleanAfterDays: days(t) })} />
      </SettingsRow>
      <SettingsRow label="Remove worktree after" description="Days after a ticket is Done or Won't do that its worktree is removed; its branch stays (0: never). Worktrees with uncommitted changes are kept.">
        <AutoSaveText aria-label="Remove worktree after (days)" inputMode="numeric" className="w-20" value={String(settings.removeAfterDays)}
          onSave={(t) => save({ removeAfterDays: days(t) })} />
      </SettingsRow>
      <SettingsRow label="On Won't do" description="When a ticket goes to Won't do or its attempt is abandoned: remove its worktree, branch (also the copy PDT pushed) and workspace at once, after some days, or keep them. Work not committed or not pushed is always kept.">
        <Select aria-label="On Won't do" className="w-48 max-w-full" options={WONT_DO_OPTIONS} value={settings.wontDo}
          onChange={(wontDo) => void save({ wontDo })} />
      </SettingsRow>
      {settings.wontDo === 'later' && (
        <SettingsRow label="Keep Won't do worktrees for" description="Days after a ticket goes to Won't do that its worktree is removed (at least 1).">
          <AutoSaveText aria-label="Keep Won't do worktrees for (days)" inputMode="numeric" className="w-20" value={String(settings.wontDoAfterDays)}
            onSave={(t) => save({ wontDoAfterDays: Math.max(1, days(t)) })} />
        </SettingsRow>
      )}
    </>
  );
}

function Worktrees({ project }: { project: Project }) {
  const list = useQuery(worktreesQuery(project.id));
  const storage = useStorage(project.id);
  const mine = storage.data?.projects.find((p) => p.projectId === project.id) ?? null;
  const byId = new Map((mine?.worktrees ?? []).map((w) => [w.id, w]));
  const description = mine && mine.worktrees.length > 0
    ? `Checkouts in .worktrees, one per branch: ${formatBytes(mine.total)}${mine.build ? `, ${formatBytes(mine.build)} of it build output` : ''}. Removing one deletes its folder.`
    : 'Checkouts in .worktrees, one per branch. Removing one deletes its folder.';
  return (
    <Section label="Worktrees" description={description}>
      {list.isPending && <Skeleton lines={3} label="Loading worktrees" />}
      {list.isError && <Alert tone="error">Worktrees could not be loaded: {list.error.message}</Alert>}
      {list.data?.length === 0 && <EmptyState>No worktrees. Running a branch makes one.</EmptyState>}
      {!!list.data?.length && (
        <ul data-flush className="flex flex-col divide-y divide-border">
          {list.data.map((w) => <WorktreeRow key={w.id} project={project} wt={w} storage={byId.get(w.id) ?? null} />)}
        </ul>
      )}
      {mine && <Cleanup project={project} settings={mine.settings} />}
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
    void invalidateProjects(qc);
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
    { label: 'Documents', node: <ProjectDocuments project={project} /> },
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
