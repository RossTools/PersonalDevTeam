// An attempt's pull request on GitHub (PDT-25), or its change request on the local forge (PDT-178): the board card's
// one-badge summary and the ticket's PR details.
import { GitPullRequest, TriangleAlert } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, Link, relativeTime, type BadgeTone } from '@ui';
import type { CiState } from '../api/gen/CiState';
import type { CiStatus } from '../api/gen/CiStatus';
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
 * True when the PR counts as merged: GitHub says so, or the attempt ended merged while the last look still saw it open
 * (git found the merge before GitHub reported it, PDT-59).
 */
const isMerged = (pr: PullRequestStatus, attemptMerged: boolean) =>
  pr.state === 'merged' || (attemptMerged && pr.state === 'open');

/**
 * The one thing to know about a PR, most urgent first: merged, conflict, CI failed, left the queue, queued, CI.
 * `attemptMerged` is the attempt's state being `merged`.
 */
export function prSummary(pr: PullRequestStatus, attemptMerged = false): { label: string; tone: BadgeTone } {
  if (isMerged(pr, attemptMerged)) return { label: 'Merged', tone: 'accent' };
  if (pr.state === 'closed') return { label: 'Closed', tone: 'neutral' };
  if (pr.mergeable === 'conflicting') return { label: 'Conflict', tone: 'failed' };
  if (pr.ci?.state === 'failed') return { label: 'CI failed', tone: 'failed' };
  if (queueFailed(pr)) return { label: 'Queue failed', tone: 'failed' };
  if (pr.inMergeQueue) return { label: 'In queue', tone: 'waiting' };
  if (pr.ci && pr.ci.state !== 'passed') return { label: CI_LABEL[pr.ci.state], tone: CI_TONE[pr.ci.state] };
  if (pr.draft) return { label: 'Draft', tone: 'neutral' };
  if (pr.ci) return { label: pr.autoMerge ? 'Auto-merge' : 'CI passed', tone: 'running' };
  return { label: pr.autoMerge ? 'Auto-merge' : 'Open', tone: 'neutral' };
}

/** When the PR was last read and, when reading it fails now, why (the rest is that older look). */
export function prFreshness(pr: PullRequestStatus): string {
  const checked = `checked ${relativeTime(pr.checkedAt)}`;
  return pr.error ? `Not updated: ${pr.error} (last ${checked})` : `Last ${checked}`;
}

/** `#12 · CI failed`, for a board card (inside its button, so not a link). A warning sign when it can't be read now. */
export function PrBadge({ pr, attemptMerged = false }: { pr: PullRequestStatus; attemptMerged?: boolean }) {
  const s = prSummary(pr, attemptMerged);
  const jobs = failedJobs(pr.ci);
  const title = `${prName(pr)}: ${pr.title}${jobs.length ? ` — failed: ${jobs.join(', ')}` : ''}\n${prFreshness(pr)}`;
  return (
    <Badge tone={s.tone} title={title} className="shrink-0">
      {pr.error ? <TriangleAlert aria-label="Not updated" /> : <GitPullRequest aria-hidden />} #{pr.number} · {s.label}
    </Badge>
  );
}

/** The ticket's PR: a link to it, then its state (CI, conflict, auto-merge, merge queue) and what failed. */
export function PrDetails({ pr, attemptMerged = false, className }: {
  pr: PullRequestStatus; attemptMerged?: boolean; className?: string;
}) {
  const jobs = failedJobs(pr.ci);
  const queueJobs = failedJobs(pr.queueCi);
  const merged = isMerged(pr, attemptMerged);
  const open = pr.state === 'open' && !merged;
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
        {open && pr.mergeable === 'conflicting' && <Badge tone="failed">Conflicts with {pr.baseBranch}</Badge>}
        {open && pr.behind && pr.mergeable !== 'conflicting' && <Badge title="The merge queue tests it on top of the base">Behind {pr.baseBranch}</Badge>}
        {open && pr.inMergeQueue && (
          <Badge tone="waiting">In merge queue{pr.queuePosition ? ` · #${pr.queuePosition}` : ''}</Badge>
        )}
        {open && !pr.inMergeQueue && pr.autoMerge && <Badge>{pr.local ? 'Merges when CI passes' : 'Auto-merge on'}</Badge>}
        {open && queueFailed(pr) && <Badge tone="failed">Left the merge queue</Badge>}
      </div>
      {open && pr.ci && pr.ci.state !== 'passed' && (
        <p className="text-meta text-muted-foreground">
          {jobs.length > 0 && <>Failed: {jobs.join(', ')} · </>}
          <Link href={prHref(pr, pr.ci.url)} external>{pr.local ? 'Log' : 'Run'}</Link>
        </p>
      )}
      {open && pr.queueCi && (pr.inMergeQueue || queueFailed(pr)) && (
        <p className="text-meta text-muted-foreground">
          Merge queue: {CI_LABEL[pr.queueCi.state].replace('CI ', '')}
          {queueJobs.length > 0 && <> ({queueJobs.join(', ')})</>} · <Link href={pr.queueCi.url} external>Run</Link>
        </p>
      )}
      <p className={cn('text-meta', pr.error ? 'text-destructive' : 'text-muted-foreground')}>{prFreshness(pr)}</p>
    </div>
  );
}

/** CI of a branch without a pull request (Buildkite builds branches), for a board card. */
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
