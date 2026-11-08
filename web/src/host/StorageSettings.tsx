// Settings › Storage (PDT-324): free disk space, what each project's worktrees take (build output apart), Clean and
// Remove, Clean all finished, and Cleanup: the built-in Clean up storage's inputs (PDT-370), its thresholds and what
// happens when a disk is almost full.
import { useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { Alert, Banner, Button, EmptyState, Section, SettingsRow, Skeleton, Switch } from '@ui';
import { api } from '../api/client';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import type { ProjectStorage } from '../api/gen/ProjectStorage';
import type { WorktreeStorage } from '../api/gen/WorktreeStorage';
import { AutoSaveRow, AutoSaveText } from '../common/AutoSave';
import { BuiltinSettingsSection, type PutInputs } from '../common/BuiltinSettings';
import { BranchChip } from '../common/BranchChip';
import { useAction } from '../common/useAction';
import { StageBadge } from '../common/StageBadge';
import { TicketKey } from '../common/TicketKey';
import { CleanButton, DiskBadge, RemoveButton, sizeText, useCleanFinished, useStorage } from '../projects/worktreeStorage';
import { formatBytes, plural } from '../common/text';
import { LoadError } from '../common/QueryStatus';
import { invalidateStorage } from '../projects/queries';
import { ticketKeys } from '../tickets/queries';

function gb(text: string): number {
  if (!/^\d+$/.test(text)) throw new Error('a whole number of GB (0 turns it off)');
  return Number(text);
}

const STORAGE_CLEANUP_KEY = ['storage-cleanup'];

/** Settings › Storage › Cleanup: the built-in Clean up storage's inputs. */
function Cleanup() {
  return (
    <BuiltinSettingsSection
      calls={api.storageCleanup}
      queryKey={STORAGE_CLEANUP_KEY}
      label="Cleanup"
      description="Every hour, finished tickets' worktrees are cleaned as each project's Worktrees settings say. When a disk with PDT's data or a project is almost full, PDT does what is set here."
      follows="Cleanup follows"
      off="finished tickets' worktrees are not cleaned on their own, and nothing pauses when a disk is almost full."
    >
      {(v, put) => <CleanupInputs v={v} put={put} />}
    </BuiltinSettingsSection>
  );
}

function CleanupInputs({ v, put }: { v: BuiltinSettingsView; put: PutInputs }) {
  const qc = useQueryClient();
  // The thresholds also set the disks' levels on this page.
  const save = async (values: Record<string, unknown>) => {
    await put(values);
    void invalidateStorage(qc);
  };
  const num = (id: string) => String(typeof v.values[id] === 'number' ? v.values[id] : '');
  return (
    <>
      <SettingsRow label="Low-disk warning" description="The Inbox warns when a disk with PDT's data or a project has less free than this (GB; 0: never).">
        <AutoSaveText aria-label="Low-disk warning (GB)" inputMode="numeric" className="w-20" value={num('lowGb')}
          onSave={async (t) => save({ lowGb: gb(t) })} />
      </SettingsRow>
      <SettingsRow label="Almost full below" description="Below this (GB; 0: never) a disk is almost full, and PDT does what is set below.">
        <AutoSaveText aria-label="Almost full below (GB)" inputMode="numeric" className="w-20" value={num('criticalGb')}
          onSave={async (t) => save({ criticalGb: gb(t) })} />
      </SettingsRow>
      <AutoSaveRow label="Pause pickup" description="While a disk is almost full, auto-start starts no new tickets, and the Board says why. Running agents carry on.">
        {(track) => <Switch aria-label="Pause pickup" checked={v.values.pausePickup === true} onChange={(on) => void track(save({ pausePickup: on }))} />}
      </AutoSaveRow>
      <AutoSaveRow label="Clean finished worktrees now" description="When a disk is almost full, remove the build output of every Done or Won't do ticket's worktree at once.">
        {(track) => <Switch aria-label="Clean finished worktrees now" checked={v.values.cleanNow === true} onChange={(on) => void track(save({ cleanNow: on }))} />}
      </AutoSaveRow>
    </>
  );
}

/** Pickup is paused (a disk almost full, or a workflow of yours): why, and Resume. */
function PausedBanner({ reason }: { reason: string }) {
  const qc = useQueryClient();
  const [resume, resuming] = useAction(async () => {
    await api.resumePickup();
    void invalidateStorage(qc);
    void qc.invalidateQueries({ queryKey: ticketKeys.lists });
  });
  return (
    <Banner tone="failed" actions={<Button size="sm" variant="secondary" pending={resuming} onClick={() => void resume()}>Resume pickup</Button>}>
      Pickup is paused: {reason}. Auto-start starts no new tickets until it resumes. Running agents carry on.
    </Banner>
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
    : `${plural(p.worktrees.length, 'worktree')}: ${formatBytes(p.total)}${p.build ? `, ${formatBytes(p.build)} of it build output` : ''}${measured < p.worktrees.length ? ' (still measuring some)' : ''}.`;
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

export function Storage() {
  const qc = useQueryClient();
  const storage = useStorage();
  const settle = () => void invalidateStorage(qc);
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
        notice={s?.pickupPause ? <PausedBanner reason={s.pickupPause.reason} /> : undefined}
      >
        {storage.isPending && <Skeleton lines={2} label="Loading storage" />}
        <LoadError what="Storage" error={storage.error} />
        {s?.disks.map((d) => (
          <SettingsRow key={d.folders.join('\n')} label={<span className="flex items-center gap-2">Disk <DiskBadge level={d.level} /></span>}
            description={<span className="font-mono">{d.folders.join(', ')}</span>}>
            <span className="text-sm">{formatBytes(d.space.free)} free of {formatBytes(d.space.total)}</span>
          </SettingsRow>
        ))}
        <SettingsRow label="Clean all finished" description="Removes the build output of every Done or Won't do ticket's worktree now.">
          <Button size="sm" pending={cleaning} onClick={() => void cleanAll()}>Clean all finished</Button>
        </SettingsRow>
      </Section>
      <Cleanup />
      {s && s.projects.length === 0 && <EmptyState>No projects with worktrees.</EmptyState>}
      {s?.projects.map((p) => <ProjectWorktrees key={p.projectId} p={p} />)}
    </>
  );
}
