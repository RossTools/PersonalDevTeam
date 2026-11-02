import { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, Link, SegmentedControl, Skeleton, toast, useConfirm } from '@ui';
import { api } from '../api/client';
import type { MergePreview } from '../api/gen/MergePreview';
import type { MergeStyle } from '../api/gen/MergeStyle';
import type { Ticket } from '../api/gen/Ticket';
import { actionError } from '../common/useAction';
import { finishMerged, patchTicket } from '../tickets/actions';
import { invalidateTickets } from '../tickets/queries';
import { invalidateWorktrees } from '../common/worktrees';

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

const styleKey = (projectId: string) => `pdt.review.mergeStyle.${projectId}`;

/** How this project last merged locally in this browser (a merge commit at first). */
function loadStyle(projectId: string): MergeStyle {
  try {
    return localStorage.getItem(styleKey(projectId)) === 'squash' ? 'squash' : 'merge';
  } catch {
    return 'merge';
  }
}

function saveStyle(projectId: string, s: MergeStyle) {
  try {
    localStorage.setItem(styleKey(projectId), s);
  } catch {
    // Storage blocked: remembered for this dialog only.
  }
}

/** Why Merge can't go ahead, as an Alert (with a button that asks the agent when it can fix it). */
function Problem({ p, onAsk, sending }: { p: MergePreview; onAsk: () => void; sending: boolean }) {
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
          action={<Button size="sm" pending={sending} onClick={onAsk}>Send to agent</Button>}
        >
          <p>{p.base} has changed the same lines in {plural(p.conflicts.length, 'file')}:</p>
          <ul className="mt-1 font-mono text-meta">{p.conflicts.map((f) => <li key={f}>{f}</li>)}</ul>
        </Alert>
      );
    case 'pr_not_mergeable':
      return p.pr?.local
        ? <Alert tone="warning">Change request #{p.pr.number} conflicts with {p.base}: merge {p.base} into {p.branch} first.</Alert>
        : <Alert tone="warning">{p.pr?.forge ?? 'The forge'} can’t merge pull request #{p.pr?.number}: it is a draft or it conflicts with {p.base}.</Alert>;
    case 'no_pr':
      return (
        <Alert
          tone="info"
          title="No pull request yet"
          action={<Button size="sm" pending={sending} onClick={onAsk}>Ask the agent to open one</Button>}
        >
          This project merges through pull requests, and {p.branch} has no open one. Once it has, Merge… sets it to merge.
        </Alert>
      );
    case 'pr_queued':
      if (p.pr?.local) return <Alert tone="info">Change request #{p.pr.number} is already set to merge; PDT merges it once its CI passes.</Alert>;
      return p.pr && !p.pr.autoMerge && !p.pr.queue
        ? <Alert tone="info">Pull request #{p.pr.number} is already set to merge; PDT merges it once its CI passes.</Alert>
        : <Alert tone="info">Pull request #{p.pr?.number} is already set to merge; {p.pr?.forge ?? 'the forge'} merges it once its checks pass.</Alert>;
    default:
      return null;
  }
}

/** Whether PDT, not the forge, holds the pull request's merge until CI passes: no merge queue, and no auto-merge (PDT-297). */
const pdtHolds = (p: MergePreview) => !!p.pr && !p.pr.local && !p.pr.queue && !p.pr.autoMerge;

/**
 * Merge…: into the base branch locally (a merge commit or a squash, remembered per project; then the ticket moves to Done and, by default, the worktree
 * goes), or, when the attempt has an open pull request, auto-merge (or its merge queue) on the project's forge; where
 * the repository allows neither, it merges now if CI passed, else PDT merges it once CI passes (by default PDT
 * finishes the ticket once it merges, PDT-75). Problems show instead of the button.
 */
export function MergeDialog({ ticket, attemptId, onClose }: { ticket: Ticket; attemptId: string; onClose: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const preview = useQuery({ queryKey: ['merge-preview', attemptId], queryFn: () => api.mergePreview(attemptId), staleTime: 0, gcTime: 0 });
  const [removeWorktree, setRemoveWorktree] = useState(true);
  const [finishOnMerge, setFinishOnMerge] = useState(true);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [style, setStyle] = useState<MergeStyle>(() => loadStyle(ticket.projectId));
  const [busy, setBusy] = useState(false);
  const [sending, setSending] = useState(false);
  const p = preview.data;

  const ask = async () => {
    setSending(true);
    try {
      await api.askAgentToFixMerge(attemptId);
      toast.success(p?.problem === 'no_pr' ? 'Asked the agent to open a pull request.' : 'Asked the agent to resolve the conflicts.');
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
      const viaForge = p.method === 'forge';
      if (!viaForge) saveStyle(ticket.projectId, style);
      const r = await api.mergeAttempt(attemptId, viaForge ? { finishOnMerge, deleteBranch: finishOnMerge && deleteBranch } : { style });
      invalidateTickets(qc, [ticket.id]);
      invalidateWorktrees(qc, ticket.projectId);
      onClose();
      if (r.outcome === 'queued') {
        const then = finishOnMerge ? `; then ${ticket.key} moves to Done and its worktree goes` : '';
        toast.success(
          p.pr?.local
            ? `Change request #${p.pr.number} merges into ${p.base} once its CI passes${then}.`
            : pdtHolds(p)
              ? `PR #${p.pr?.number} is set to merge. PDT merges it once its CI passes${then}.`
              : p.pr?.queue
                ? `PR #${p.pr.number} is set to merge through the ${p.pr.queueName}${then}.`
                : `Auto-merge is on for PR #${p.pr?.number}. ${p.pr?.forge ?? 'The forge'} merges it once the checks pass${then}.`,
        );
        return;
      }
      if (viaForge) {
        // Merged at once (CI had passed, or the forge merged it straight away): PDT finishes the ticket as asked.
        toast.success(`Merged ${p.pr?.local ? 'change request' : 'PR'} #${p.pr?.number} into ${p.base}.`);
        return;
      }
      const fresh = await api.getTicket(ticket.id);
      if (removeWorktree) await finishMerged(confirm, qc, fresh, { deleteBranch });
      else {
        await patchTicket(qc, ticket.id, { stage: 'done' });
        toast.success(`${style === 'squash' ? 'Squashed' : 'Merged'} ${p.branch} into ${p.base}; ${ticket.key} moved to Done.`);
      }
    } catch (e) {
      toast.error(actionError(e));
      void preview.refetch();
    } finally {
      setBusy(false);
    }
  };

  const viaForge = p?.method === 'forge';
  const queueName = p?.pr?.queueName ?? 'merge queue';
  return (
    <Dialog
      open
      onOpenChange={(o) => { if (!o) onClose(); }}
      title={p?.pr ? `Merge ${p.pr.local ? 'change request' : 'pull request'} #${p.pr.number}` : `Merge ${ticket.key}`}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          {p && !p.problem && (
            <Button variant="primary" pending={busy} onClick={() => void merge()}>
              {viaForge
                ? (p.pr?.queue ? `Add to ${queueName}` : p.pr?.local || pdtHolds(p) ? 'Merge when CI passes' : 'Turn on auto-merge')
                : style === 'squash' ? 'Squash and merge' : 'Merge'}
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
          {viaForge && p.pr?.local ? (
            <p>
              Change request #{p.pr.number} merges into <span className="font-mono">{p.base}</span> here (a fast-forward, or a merge commit
              when {p.base} has moved) once its CI passes.
              {finishOnMerge ? '' : ` PDT notices the merge and offers to move ${ticket.key} to Done.`}
            </p>
          ) : viaForge && p.pr ? (
            <p>
              <Link href={p.pr!.url} external>Pull request #{p.pr!.number}</Link> merges into <span className="font-mono">{p.base}</span> on{' '}
              {p.pr!.forge}{p.pr!.queue ? ` through the ${queueName}` : ' once its checks pass'}.
              {pdtHolds(p) ? ` The repository doesn’t allow auto-merge, so PDT merges it now if its CI passed, or once it passes.` : ''}
              {finishOnMerge ? '' : ` PDT notices the merge and offers to move ${ticket.key} to Done.`}
            </p>
          ) : (
            !viaForge && p.problem !== 'inactive' && (
              <p>
                Merge <span className="font-mono">{p.branch}</span>
                {p.commits > 0 ? ` (${plural(p.commits, 'commit')})` : ''} into <span className="font-mono">{p.base}</span>
                {style === 'squash' ? ' as one new commit' : ' with a merge commit'}, then move {ticket.key} to Done.
              </p>
            )
          )}
          <Problem p={p} onAsk={() => void ask()} sending={sending} />
          {viaForge && !p.problem && (
            <div className="flex flex-col gap-1.5">
              <Checkbox checked={finishOnMerge} onChange={(v) => { setFinishOnMerge(v); if (!v) setDeleteBranch(false); }}>
                Move to Done and clean up once it merges
              </Checkbox>
              <Checkbox checked={deleteBranch} disabled={!finishOnMerge} onChange={setDeleteBranch}>
                Delete the branch
              </Checkbox>
            </div>
          )}
          {!viaForge && !p.problem && (
            <div className="flex flex-col gap-1.5">
              <SegmentedControl<MergeStyle>
                aria-label="Merge as"
                className="mb-1 self-start"
                options={[{ value: 'merge', label: 'Merge commit' }, { value: 'squash', label: 'Squash' }]}
                value={style}
                onChange={setStyle}
              />
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
