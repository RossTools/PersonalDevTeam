import { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, Link, Skeleton, toast, useConfirm } from '@ui';
import { api } from '../api/client';
import type { MergePreview } from '../api/gen/MergePreview';
import type { Ticket } from '../api/gen/Ticket';
import { actionError } from '../common/useAction';
import { finishMerged, patchTicket } from '../tickets/actions';
import { invalidateTickets } from '../tickets/queries';

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/** Why Merge can't go ahead, as an Alert (with Send to agent for a conflict). */
function Problem({ p, onSendConflicts, sending }: { p: MergePreview; onSendConflicts: () => void; sending: boolean }) {
  switch (p.problem) {
    case 'inactive':
      return <Alert tone="info">This attempt has ended.</Alert>;
    case 'dirty':
      return <Alert tone="warning" title="Uncommitted changes">{p.branch} has changes that aren’t committed. Ask the agent to commit them (or discard them) first.</Alert>;
    case 'nothing':
      return <Alert tone="info">{p.branch} has no commits that {p.base} doesn’t already have.</Alert>;
    case 'base_dirty':
      return (
        <Alert tone="warning" title={`${p.base} has uncommitted changes`}>
          The checkout at <span className="font-mono">{p.baseCheckout}</span> has {p.base} checked out with changes that aren’t committed. Commit or stash them first.
        </Alert>
      );
    case 'conflict':
      return (
        <Alert
          tone="warning"
          title="The merge conflicts"
          action={<Button size="sm" pending={sending} onClick={onSendConflicts}>Send to agent</Button>}
        >
          <p>{p.base} has changed the same lines in {plural(p.conflicts.length, 'file')}:</p>
          <ul className="mt-1 font-mono text-meta">{p.conflicts.map((f) => <li key={f}>{f}</li>)}</ul>
        </Alert>
      );
    case 'pr_not_mergeable':
      return <Alert tone="warning">GitHub can’t merge pull request #{p.pr?.number}: it is a draft or it conflicts with {p.base}.</Alert>;
    case 'pr_queued':
      return <Alert tone="info">Auto-merge is already on for pull request #{p.pr?.number}; GitHub merges it once its checks pass.</Alert>;
    default:
      return null;
  }
}

/**
 * Merge…: into the base branch locally (a merge commit; then the ticket moves to Done and, by default, the worktree
 * goes), or, when the attempt has an open pull request, auto-merge on GitHub. Problems show instead of the button.
 */
export function MergeDialog({ ticket, attemptId, onClose }: { ticket: Ticket; attemptId: string; onClose: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const preview = useQuery({ queryKey: ['merge-preview', attemptId], queryFn: () => api.mergePreview(attemptId), staleTime: 0, gcTime: 0 });
  const [removeWorktree, setRemoveWorktree] = useState(true);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [busy, setBusy] = useState(false);
  const [sending, setSending] = useState(false);
  const p = preview.data;

  const sendConflicts = async () => {
    setSending(true);
    try {
      await api.sendConflicts(attemptId);
      toast.success('Asked the agent to resolve the conflicts.');
      onClose();
    } catch (e) {
      toast.error(actionError(e));
    } finally {
      setSending(false);
    }
  };

  const merge = async () => {
    if (!p) return;
    setBusy(true);
    try {
      const r = await api.mergeAttempt(attemptId);
      invalidateTickets(qc, [ticket.id]);
      onClose();
      if (r.outcome === 'queued') {
        toast.success(`Auto-merge is on for PR #${p.pr?.number}. GitHub merges it once the checks pass.`);
        return;
      }
      const fresh = await api.getTicket(ticket.id);
      if (removeWorktree) await finishMerged(confirm, qc, fresh, { deleteBranch });
      else {
        await patchTicket(qc, ticket.id, { stage: 'done' });
        toast.success(`Merged ${p.branch} into ${p.base}; ${ticket.key} moved to Done.`);
      }
    } catch (e) {
      toast.error(actionError(e));
      void preview.refetch();
    } finally {
      setBusy(false);
    }
  };

  const github = p?.method === 'github';
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      title={github ? `Merge pull request #${p?.pr?.number}` : `Merge ${ticket.key}`}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          {p && !p.problem && (
            <Button variant="primary" pending={busy} onClick={() => void merge()}>
              {github ? (p.pr?.queue ? 'Add to merge queue' : 'Turn on auto-merge') : 'Merge'}
            </Button>
          )}
        </>
      }
    >
      {preview.isPending ? (
        <Skeleton lines={3} label="Checking the merge" />
      ) : !p ? (
        <Alert tone="error">{preview.error ? actionError(preview.error) : 'Could not check the merge.'}</Alert>
      ) : (
        <div className="flex flex-col gap-3 text-sm">
          {github ? (
            <p>
              <Link href={p.pr!.url} external>Pull request #{p.pr!.number}</Link> merges into <span className="font-mono">{p.base}</span> on
              GitHub{p.pr!.queue ? ' through the merge queue' : ' once its checks pass'}. PDT notices the merge and offers to move {ticket.key} to Done.
            </p>
          ) : (
            p.problem !== 'inactive' && (
              <p>
                Merge <span className="font-mono">{p.branch}</span> into <span className="font-mono">{p.base}</span> with a merge commit
                {p.commits > 0 ? ` (${plural(p.commits, 'commit')})` : ''}, then move {ticket.key} to Done.
              </p>
            )
          )}
          <Problem p={p} onSendConflicts={() => void sendConflicts()} sending={sending} />
          {!github && !p.problem && (
            <div className="flex flex-col gap-1.5">
              <Checkbox checked={removeWorktree} onChange={(v) => { setRemoveWorktree(v); if (!v) setDeleteBranch(false); }}>
                Remove the worktree
              </Checkbox>
              <Checkbox checked={deleteBranch} disabled={!removeWorktree} onChange={setDeleteBranch}>
                Delete the branch
              </Checkbox>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
}
