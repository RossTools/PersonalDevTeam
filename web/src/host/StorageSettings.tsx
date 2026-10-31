// Settings › Storage (PDT-324): free disk space, what each project's worktrees take (build output apart), Clean and
// Remove, Clean all finished, and the low-disk thresholds.
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { Alert, Banner, Button, EmptyState, Section, SettingsRow, Skeleton } from '@ui';
import { api } from '../api/client';
import type { HostSettingsPatch } from '../api/gen/HostSettingsPatch';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { ProjectStorage } from '../api/gen/ProjectStorage';
import type { WorktreeStorage } from '../api/gen/WorktreeStorage';
import { AutoSaveText } from '../common/AutoSave';
import { BranchChip } from '../common/BranchChip';
import { useAction } from '../common/useAction';
import { TicketKey } from '../common/TicketKey';
import { CleanButton, DiskBadge, formatBytes, RemoveButton, sizeText, StageBadge, useCleanFinished, useStorage } from '../projects/worktreeStorage';

function gb(text: string): number {
  if (!/^\d+$/.test(text)) throw new Error('a whole number of GB (0 turns it off)');
  return Number(text);
}

function Thresholds({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const m = useMutation({
    mutationFn: (p: HostSettingsPatch) => api.saveHostSettings(p),
    onSuccess: (view) => {
      qc.setQueryData(['host-settings'], view);
      void qc.invalidateQueries({ queryKey: ['storage'] });
    },
  });
  const s = v.settings;
  return (
    <>
      <SettingsRow label="Low-disk warning" description="The Inbox warns when a disk with PDT's data or a project has less free than this (GB; 0: never).">
        <AutoSaveText aria-label="Low-disk warning (GB)" inputMode="numeric" className="w-20" value={String(s.lowDiskGb)}
          onSave={async (t) => m.mutateAsync({ lowDiskGb: gb(t) })} />
      </SettingsRow>
      <SettingsRow label="Pause auto-start below" description="Below this (GB; 0: never), auto-start starts no new tickets until space is freed. Running agents carry on.">
        <AutoSaveText aria-label="Pause auto-start below (GB)" inputMode="numeric" className="w-20" value={String(s.criticalDiskGb)}
          onSave={async (t) => m.mutateAsync({ criticalDiskGb: gb(t) })} />
      </SettingsRow>
    </>
  );
}

function WorktreeLine({ wt }: { wt: WorktreeStorage }) {
  return (
    <li className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <BranchChip branch={wt.branch} forge={null} />
        {wt.ticket && <TicketKey ticketKey={wt.ticket.key} />}
        {wt.ticket && <StageBadge stage={wt.ticket.stage} />}
        <span className="flex-1" />
        <span className="text-xs text-muted-foreground">{sizeText(wt.size, wt.missing)}</span>
        {!wt.missing && <CleanButton id={wt.id} label="Clean" />}
        {!wt.missing && (!wt.ticket || wt.ticket.finished) && <RemoveButton wt={wt} size={wt.size} />}
      </div>
      {wt.ticket && <span className="truncate text-meta text-muted-foreground" title={wt.ticket.title}>{wt.ticket.title}</span>}
    </li>
  );
}

function ProjectWorktrees({ p }: { p: ProjectStorage }) {
  const out = p.buildOutput;
  const measured = p.worktrees.filter((w) => w.size).length;
  const description = p.worktrees.length === 0
    ? 'No worktrees.'
    : `${p.worktrees.length} worktree${p.worktrees.length === 1 ? '' : 's'}: ${formatBytes(p.total)}${p.build ? `, ${formatBytes(p.build)} of it build output` : ''}${measured < p.worktrees.length ? ' (still measuring some)' : ''}.`;
  return (
    <Section label={p.name} description={description}>
      <SettingsRow
        label="Build output"
        description={out.detected
          ? 'Detected from the project’s files. Declare your own as [storage] build_output in .pdt/project.toml.'
          : 'From [storage] build_output in .pdt/project.toml. Only what git ignores is removed.'}
      >
        <span className="min-w-0 truncate font-mono text-xs text-muted-foreground">{out.paths.length ? out.paths.join(', ') : 'none'}</span>
      </SettingsRow>
      {out.error && <Alert tone="error">{out.error}</Alert>}
      {p.worktrees.length > 0 && (
        <ul data-flush className="flex flex-col divide-y divide-border">
          {p.worktrees.map((w) => <WorktreeLine key={w.id} wt={w} />)}
        </ul>
      )}
    </Section>
  );
}

export function Storage({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const storage = useStorage();
  const settle = () => void qc.invalidateQueries({ queryKey: ['storage'] });
  const [cleanAll, cleaning] = useCleanFinished();
  const [refresh, refreshing] = useAction(async () => {
    await api.refreshStorage();
    settle();
  });
  const s = storage.data;
  return (
    <>
      <Section
        label="Storage"
        description="Free disk space, and what each worktree takes. Finished tickets' build output is cleaned on its own (each project's Worktrees settings say when)."
        actions={<Button size="sm" variant="ghost" pending={refreshing || !!s?.measuring} onClick={() => void refresh()}><RefreshCw aria-hidden /> Measure again</Button>}
        notice={s?.autoStartPaused ? <Banner tone="failed">A disk is almost full: auto-start starts no new tickets until there is more space. Running agents carry on.</Banner> : undefined}
      >
        {storage.isPending && <Skeleton lines={2} label="Loading storage" />}
        {storage.isError && <Alert tone="error">Storage could not be loaded: {storage.error.message}</Alert>}
        {s?.disks.map((d) => (
          <SettingsRow key={d.folders.join('\n')} label={<span className="flex items-center gap-2">Disk <DiskBadge level={d.level} /></span>}
            description={<span className="font-mono">{d.folders.join(', ')}</span>}>
            <span className="text-sm">{formatBytes(d.space.free)} free of {formatBytes(d.space.total)}</span>
          </SettingsRow>
        ))}
        <SettingsRow label="Clean all finished" description="Removes the build output of every Done or Won't do ticket's worktree now.">
          <Button size="sm" pending={cleaning} onClick={() => void cleanAll()}>Clean all finished</Button>
        </SettingsRow>
        <Thresholds v={v} />
      </Section>
      {s && s.projects.length === 0 && <EmptyState>No projects with worktrees.</EmptyState>}
      {s?.projects.map((p) => <ProjectWorktrees key={p.projectId} p={p} />)}
    </>
  );
}
