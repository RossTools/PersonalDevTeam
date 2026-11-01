// An attempt's pull request on its forge (PDT-25), or its change request on the local forge (PDT-178): the board card's
// one-badge summary and the ticket's PR details.
import { GitPullRequest, TriangleAlert } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, Link, relativeTime, type BadgeTone } from '@ui';
import type { CiState } from '../api/gen/CiState';
import type { CiStatus } from '../api/gen/CiStatus';
import type { MergeUpdateStatus } from '../api/gen/MergeUpdateStatus';
import type { PullRequestStatus } from '../api/gen/PullRequestStatus';

const CI_LABEL: Record<CiState, string> = {
  passed: 'CI passed', failed: 'CI failed', running: 'CI running', queued: 'CI queued', cancelled: 'CI cancelled',
};
const CI_TONE: Record<CiState, BadgeTone> = {
  passed: 'running', failed: 'failed', running: 'waiting', queued: 'waiting', cancelled: 'neutral',
};

/** The failed jobs of a CI result, for titles and the details line. */
export function failedJobs(ci: CiStatus | null): string[] {
  return ci ? ci.runs.flatMap((r) => r.failedJobs) : [];
}

/** `PR #12`, or `Change #3` on the local forge. */
export const prName = (pr: PullRequestStatus) => `${pr.local ? 'Change' : 'PR'} #${pr.number}`;

/**
 * Where a link of the PR goes. The local forge's (its CI logs) name pdtd at 127.0.0.1 for agents; the UI opens them
 * on its own origin, so they work remotely too.
 */
export const prHref = (pr: PullRequestStatus, url: string) => (pr.local ? url.replace(/^https?:\/\/[^/]+/, '') : url);

/** True when the PR left the merge queue because its queue run failed (and has not been re-queued). */
const queueFailed = (pr: PullRequestStatus) => pr.state === 'open' && !pr.inMergeQueue && pr.queueCi?.state === 'failed';

/**
 * True when the PR counts as merged: the forge says so, or the attempt ended merged while the last look still saw it
 * open (git found the merge before the forge reported it, PDT-59).
 */
const isMerged = (pr: PullRequestStatus, attemptMerged: boolean) =>
  pr.state === 'merged' || (attemptMerged && pr.state === 'open');

/** Where the attempt is with its base's conflicts (fixed once, at merge time), as a badge. */
export function mergeUpdateBadge(m: MergeUpdateStatus, base: string): { label: string; tone: BadgeTone; title: string } {
  switch (m) {
    case 'deferred':
      return { label: `Conflicts with ${base} (deferred)`, tone: 'neutral', title: 'Resolved at merge time: the agent isn’t interrupted now' };
    case 'updating':
      return { label: `Updating with ${base}…`, tone: 'waiting', title: `Merging ${base} into the branch, with no agent` };
    case 'resolving':
      return { label: 'Resolving conflicts (agent)', tone: 'waiting', title: 'An agent resolves the conflicts that couldn’t be resolved automatically' };
  }
}

/**
 * The one thing to know about a PR, most urgent first: merged, being updated with its base, conflict, CI failed, left
 * the queue, queued, CI. `attemptMerged` is the attempt's state being `merged`; `mergeUpdate` the attempt's.
 */
export function prSummary(
  pr: PullRequestStatus, attemptMerged = false, mergeUpdate: MergeUpdateStatus | null = null,
): { label: string; tone: BadgeTone } {
  if (isMerged(pr, attemptMerged)) return { label: 'Merged', tone: 'accent' };
  if (pr.state === 'closed') return { label: 'Closed', tone: 'neutral' };
  if (mergeUpdate === 'updating') return { label: 'Updating', tone: 'waiting' };
  if (mergeUpdate === 'resolving') return { label: 'Resolving conflicts', tone: 'waiting' };
  if (pr.mergeable === 'conflicting') {
    return mergeUpdate === 'deferred' ? { label: 'Conflict (deferred)', tone: 'neutral' } : { label: 'Conflict', tone: 'failed' };
  }
  if (pr.ci?.state === 'failed') return { label: 'CI failed', tone: 'failed' };
  if (queueFailed(pr)) return { label: 'Queue failed', tone: 'failed' };
  if (pr.inMergeQueue) return { label: 'In queue', tone: 'waiting' };
  if (pr.ci && pr.ci.state !== 'passed') return { label: CI_LABEL[pr.ci.state], tone: CI_TONE[pr.ci.state] };
  if (pr.draft) return { label: 'Draft', tone: 'neutral' };
  if (pr.ci) return { label: pr.autoMerge ? 'Auto-merge' : pr.mergeHeld ? 'Merging' : 'CI passed', tone: 'running' };
  return { label: pr.autoMerge ? 'Auto-merge' : pr.mergeHeld ? 'Set to merge' : 'Open', tone: 'neutral' };
}

/** When the PR was last read and, when reading it fails now, why (the rest is that older look). */
export function prFreshness(pr: PullRequestStatus): string {
  const checked = `checked ${relativeTime(pr.checkedAt)}`;
  return pr.error ? `Not updated: ${pr.error} (last ${checked})` : `Last ${checked}`;
}

/** `#12 · CI failed`, for a board card (inside its button, so not a link). A warning sign when it can't be read now. */
export function PrBadge({ pr, attemptMerged = false, mergeUpdate = null }: {
  pr: PullRequestStatus; attemptMerged?: boolean; mergeUpdate?: MergeUpdateStatus | null;
}) {
  const s = prSummary(pr, attemptMerged, mergeUpdate);
  const jobs = failedJobs(pr.ci);
  const title = `${prName(pr)}: ${pr.title}${jobs.length ? ` — failed: ${jobs.join(', ')}` : ''}\n${prFreshness(pr)}`;
  return (
    <Badge tone={s.tone} title={title} className="shrink-0">
      {pr.error ? <TriangleAlert aria-label="Not updated" /> : <GitPullRequest aria-hidden />} #{pr.number} · {s.label}
    </Badge>
  );
}

/**
 * The ticket's PR: a link to it, then its state (CI, conflict, auto-merge, merge queue, an update with its base) and what
 * failed. `queueName` is what the project's forge calls its merge queue; `mergeUpdate` the attempt's.
 */
export function PrDetails({ pr, attemptMerged = false, queueName = 'merge queue', mergeUpdate = null, className }: {
  pr: PullRequestStatus; attemptMerged?: boolean; queueName?: string; mergeUpdate?: MergeUpdateStatus | null; className?: string;
}) {
  const Queue = queueName.charAt(0).toUpperCase() + queueName.slice(1);
  const jobs = failedJobs(pr.ci);
  const queueJobs = failedJobs(pr.queueCi);
  const merged = isMerged(pr, attemptMerged);
  const open = pr.state === 'open' && !merged;
  const update = mergeUpdate ? mergeUpdateBadge(mergeUpdate, pr.baseBranch) : null;
  return (
    <div className={cn('flex min-w-0 flex-col gap-1 text-xs', className)} aria-label="Pull request">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        {pr.url ? (
          <Link href={prHref(pr, pr.url)} external className="min-w-0 truncate" title={pr.title}>
            <GitPullRequest aria-hidden className="size-3.5 shrink-0" /> {prName(pr)}
          </Link>
        ) : (
          <span className="inline-flex min-w-0 items-center gap-1 truncate" title={pr.title}>
            <GitPullRequest aria-hidden className="size-3.5 shrink-0" /> {prName(pr)}
          </span>
        )}
        {merged && <Badge tone="accent">Merged</Badge>}
        {pr.state === 'closed' && <Badge>Closed</Badge>}
        {open && pr.draft && <Badge>Draft</Badge>}
        {open && pr.ci && <Badge tone={CI_TONE[pr.ci.state]}>{CI_LABEL[pr.ci.state]}</Badge>}
        {open && update && <Badge tone={update.tone} title={update.title}>{update.label}</Badge>}
        {open && !update && pr.mergeable === 'conflicting' && <Badge tone="failed">Conflicts with {pr.baseBranch}</Badge>}
        {open && pr.behind && pr.mergeable !== 'conflicting' && <Badge title={`The ${queueName} tests it on top of the base`}>Behind {pr.baseBranch}</Badge>}
        {open && pr.inMergeQueue && (
          <Badge tone="waiting">In {queueName}{pr.queuePosition ? ` · #${pr.queuePosition}` : ''}</Badge>
        )}
        {open && !pr.inMergeQueue && pr.autoMerge && <Badge>{pr.local ? 'Merges when CI passes' : 'Auto-merge on'}</Badge>}
        {open && pr.mergeHeld && <Badge title="The repository doesn’t allow auto-merge, so PDT merges it once CI passes">Merging when CI passes</Badge>}
        {open && queueFailed(pr) && <Badge tone="failed">Left the {queueName}</Badge>}
      </div>
      {open && pr.ci && pr.ci.state !== 'passed' && (
        <p className="text-meta text-muted-foreground">
          {jobs.length > 0 && <>Failed: {jobs.join(', ')} · </>}
          <Link href={prHref(pr, pr.ci.url)} external>{pr.local ? 'Log' : 'Run'}</Link>
        </p>
      )}
      {open && pr.queueCi && (pr.inMergeQueue || queueFailed(pr)) && (
        <p className="text-meta text-muted-foreground">
          {Queue}: {CI_LABEL[pr.queueCi.state].replace('CI ', '')}
          {queueJobs.length > 0 && <> ({queueJobs.join(', ')})</>} · <Link href={pr.queueCi.url} external>Run</Link>
        </p>
      )}
      <p className={cn('text-meta', pr.error ? 'text-destructive' : 'text-muted-foreground')}>{prFreshness(pr)}</p>
    </div>
  );
}

/** CI of a branch without a pull request (a CI plugin that builds branches), for a board card. */
export function CiBadge({ ci }: { ci: CiStatus }) {
  const jobs = failedJobs(ci);
  return (
    <Badge tone={CI_TONE[ci.state]} title={jobs.length ? `Failed: ${jobs.join(', ')}` : undefined} className="shrink-0">
      {CI_LABEL[ci.state]}
    </Badge>
  );
}

/** CI of the attempt's branch without a pull request: its state, what failed, and the build. */
export function CiDetails({ ci, className }: { ci: CiStatus; className?: string }) {
  const jobs = failedJobs(ci);
  return (
    <div className={cn('flex min-w-0 flex-wrap items-center gap-1.5 text-xs', className)} aria-label="CI">
      <Badge tone={CI_TONE[ci.state]}>{CI_LABEL[ci.state]}</Badge>
      {jobs.length > 0 && <span className="text-meta text-muted-foreground">Failed: {jobs.join(', ')}</span>}
      {ci.url && <Link href={ci.url} external className="text-meta">Build</Link>}
    </div>
  );
}
