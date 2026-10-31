// What worktrees take on disk (PDT-324), shared by Settings › Storage and a project's Worktrees list: sizes as last
// measured, Clean build output, and Remove with a warning for work that only this checkout holds.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Button, toast, useConfirm, type BadgeTone } from '@ui';
import { Eraser, Trash2 } from 'lucide-react';
import { api, ApiError } from '../api/client';
import type { DiskLevel } from '../api/gen/DiskLevel';
import type { TicketStage } from '../api/gen/TicketStage';
import type { WorktreeSize } from '../api/gen/WorktreeSize';
import { useAction } from '../common/useAction';

/** `bytes` for people: "512 MB", "1.5 GB" (as the server's Inbox items say it). */
export function formatBytes(bytes: number): string {
  const units = ['bytes', 'KB', 'MB', 'GB', 'TB'];
  let v = bytes;
  let unit = 0;
  while (v >= 1024 && unit < units.length - 1) {
    v /= 1024;
    unit += 1;
  }
  return unit === 0 || v >= 10 ? `${Math.round(v)} ${units[unit]}` : `${v.toFixed(1)} ${units[unit]}`;
}

/** "3.2 GB, 2.9 GB of it build output", or why there is no size yet. */
export function sizeText(size: WorktreeSize | null, missing = false): string {
  if (missing) return 'Folder missing';
  if (!size) return 'Not measured yet';
  return size.build > 0 ? `${formatBytes(size.total)}, ${formatBytes(size.build)} of it build output` : formatBytes(size.total);
}

/** What only this checkout holds, in a sentence, or null when nothing would be lost with it. */
export function atRisk(size: WorktreeSize | null): string | null {
  if (!size) return null;
  const parts = [
    size.uncommitted ? 'uncommitted changes' : null,
    size.unpushed > 0 ? `${size.unpushed} commit${size.unpushed === 1 ? '' : 's'} its remote doesn't have` : null,
    size.neverPushed ? 'a branch that was never pushed' : null,
  ].filter(Boolean);
  return parts.length ? `It has ${parts.join(' and ')}.` : null;
}

const STAGE_LABEL: Record<TicketStage, string> = {
  draft: 'Draft', ready: 'Ready', todo: 'To do', in_progress: 'In progress', done: 'Done', wont_do: "Won't do",
};

export function StageBadge({ stage }: { stage: TicketStage }) {
  const tone: BadgeTone = stage === 'in_progress' ? 'running' : 'neutral';
  return <Badge tone={tone}>{STAGE_LABEL[stage]}</Badge>;
}

export function DiskBadge({ level }: { level: DiskLevel }) {
  if (level === 'ok') return null;
  return <Badge tone={level === 'critical' ? 'failed' : 'waiting'}>{level === 'critical' ? 'Almost full' : 'Low'}</Badge>;
}

/** Every project's storage (or one project's), refreshed while it is on screen. */
export function useStorage(project?: string) {
  return useQuery({ queryKey: ['storage', project ?? null], queryFn: () => api.storage(project), refetchInterval: 5000 });
}

function useSettle() {
  const qc = useQueryClient();
  return () => {
    void qc.invalidateQueries({ queryKey: ['storage'] });
    void qc.invalidateQueries({ queryKey: ['worktrees'] });
    void qc.invalidateQueries({ queryKey: ['sessions'] });
    void qc.invalidateQueries({ queryKey: ['workspaces'] });
  };
}

/** Clean build output: removes what the project counts as build output, never tracked files or work. */
export function CleanButton({ id, label = 'Clean build output' }: { id: string; label?: string }) {
  const settle = useSettle();
  const [clean, cleaning] = useAction(async () => {
    try {
      const r = await api.cleanWorktree(id);
      toast.success(r.removed.length ? `Freed ${formatBytes(r.freed)}` : 'No build output to remove');
    } finally {
      settle();
    }
  });
  return (
    <Button size="sm" pending={cleaning} onClick={() => void clean()}>
      <Eraser aria-hidden /> {label}
    </Button>
  );
}

/** Remove worktree…: confirms, saying what would be lost, and asks again before discarding uncommitted changes. */
export function useRemoveWorktree(wt: { id: string; branch: string }, size: WorktreeSize | null) {
  const confirm = useConfirm();
  const settle = useSettle();
  return useAction(async () => {
    const risk = atRisk(size);
    const first = await confirm({
      title: `Remove worktree ${wt.branch}?`,
      body: `The worktree folder is deleted and its ended sessions are forgotten.${risk ? ` ${risk} Its commits stay on the branch unless you delete it too.` : ''}`,
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
}

export function RemoveButton({ wt, size }: { wt: { id: string; branch: string }; size: WorktreeSize | null }) {
  const [remove, removing] = useRemoveWorktree(wt, size);
  return (
    <Button size="sm" pending={removing} onClick={() => void remove()}>
      <Trash2 aria-hidden /> Remove…
    </Button>
  );
}

/** Clean all finished: the build output of every Done or Won't do ticket's worktree, now. */
export function useCleanFinished() {
  const settle = useSettle();
  return useAction(async () => {
    try {
      const r = await api.cleanFinished();
      toast.success(r.cleaned ? `Cleaned ${r.cleaned} worktree${r.cleaned === 1 ? '' : 's'}, freed ${formatBytes(r.freed)}` : 'No finished ticket’s worktree had build output');
      if (r.skipped.length) toast.info(`Skipped: ${r.skipped.join('; ')}`);
    } finally {
      settle();
    }
  });
}

/** Settings › Storage. */
export const STORAGE_PATH = '/settings/host/storage';
