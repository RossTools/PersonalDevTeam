import { FileDiff } from 'lucide-react';
import { Button, DiffStat } from '@ui';
import { useOpenChanges } from './open';
import { useDiffSize } from './queries';

/** "Changes +120 −30" in a header: opens the worktree's changes tab. */
export function ChangesButton({ worktreeId, projectId }: { worktreeId: string; projectId: string }) {
  const openChanges = useOpenChanges();
  const size = useDiffSize(worktreeId);
  return (
    <Button size="sm" variant="ghost" onClick={() => openChanges({ id: worktreeId, projectId })} title="Review the changes on this branch">
      <FileDiff aria-hidden /> Changes
      {size && size.files > 0 && <DiffStat added={size.added} removed={size.removed} className="text-xs" />}
    </Button>
  );
}
