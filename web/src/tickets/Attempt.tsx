// A ticket's attempt (branch, state, sessions) and the banner offered once its branch is merged: shared by the
// ticket tab and the peek.
import { useQueryClient } from '@tanstack/react-query';
import { GitMerge } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Badge, Banner, Button, useConfirm, type BadgeTone } from '@ui';
import type { AttemptSession } from '../api/gen/AttemptSession';
import type { AttemptState } from '../api/gen/AttemptState';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import { AgentDot, agentState, itemState, sessionUnseen } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { useAction } from '../common/useAction';
import { abandonAttempt, cleanUpAttempt, finishMerged, useOpenSession } from './actions';
import { isClosed } from './model';
import { useAllSessions, useProjectForge } from './queries';
import { CiDetails, PrDetails } from './PullRequest';
import { OverviewSection } from './TicketParts';

const STATE_LABEL: Record<AttemptState, string> = { active: 'Active', merged: 'Merged', abandoned: 'Abandoned' };
const STATE_TONE: Record<AttemptState, BadgeTone> = { active: 'running', merged: 'accent', abandoned: 'neutral' };

export function AttemptBadge({ state }: { state: AttemptState }) {
  return <Badge tone={STATE_TONE[state]}>{STATE_LABEL[state]}</Badge>;
}

/** The live state of an attempt's session when the sessions list has it, else from the ticket's own copy. */
function sessionState(s: AttemptSession, live: Session | undefined) {
  if (live) return itemState({ session: live, unseen: live.unseen });
  return agentState({ kind: 'agent_tui', status: s.status });
}

/**
 * The latest attempt: branch, state, its sessions (click opens the tab), and Abandon… while it is active (Clean up…
 * once it is abandoned with its worktree kept). With no
 * attempt, the sessions workflow steps started on the ticket (a Refiner's), if any.
 */
export function AttemptSection({ ticket: t, compact = false }: { ticket: Ticket; compact?: boolean }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [abandon, abandoning] = useAction(() => abandonAttempt(confirm, qc, t, t.attempt!));
  const [cleanUp, cleaning] = useAction(() => cleanUpAttempt(confirm, qc, t, t.attempt!));
  const queueName = useProjectForge(t.attempt?.pr ? t.projectId : null).data?.queueName ?? undefined;
  const a = t.attempt;
  if (!a) {
    return t.sessions.length > 0 ? <OverviewSection title="Sessions"><SessionList ticket={t} sessions={t.sessions} /></OverviewSection> : null;
  }
  // Compact: this attempt's sessions and the steps' ones on the ticket (no attempt), not earlier attempts'.
  const sessions = compact ? t.sessions.filter((s) => s.attemptId === null || s.attemptId === a.id) : t.sessions;
  return (
    <OverviewSection
      title="Attempt"
      actions={a.state === 'active' ? (
        <Button variant="ghost" size="sm" compact pending={abandoning} onClick={() => void abandon()}>Abandon…</Button>
      ) : a.state === 'abandoned' && a.worktreeId !== null && (
        <Button variant="ghost" size="sm" compact pending={cleaning} onClick={() => void cleanUp()}>Clean up…</Button>
      )}
    >
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <span className="min-w-0 flex-1 truncate font-mono" title={a.branch}>{a.branch}</span>
        <AttemptBadge state={a.state} />
      </div>
      {a.pr && <PrDetails pr={a.pr} attemptMerged={a.state === 'merged'} mergeUpdate={a.mergeUpdate ?? null} {...(queueName ? { queueName } : {})} />}
      {!a.pr && a.ci && <CiDetails ci={a.ci} />}
      {!compact && <p className="text-meta text-muted-foreground">Base {a.baseBranch}{t.attempts.length > 1 && ` · attempt ${t.attempts.length}`}</p>}
      {sessions.length > 0 ? (
        <SessionList ticket={t} sessions={sessions} />
      ) : (
        <p className="text-xs text-muted-foreground">No sessions.</p>
      )}
    </OverviewSection>
  );
}

/** A ticket's sessions, each opening its tab on click. */
function SessionList({ ticket: t, sessions }: { ticket: Ticket; sessions: AttemptSession[] }) {
  const openSession = useOpenSession();
  const all = useAllSessions();
  return (
    <ul aria-label="Sessions" className="flex flex-col gap-0.5">
      {sessions.map((s) => {
        const live = all.find((x) => x.id === s.id);
        const state = sessionState(s, live);
        return (
          <li key={s.id}>
            <button
              type="button"
              onClick={() => openSession({ id: s.id, projectId: t.projectId })}
              className={cn(
                'flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-0.5 text-left text-xs outline-none hover:bg-muted',
                'focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch',
              )}
            >
              <AgentDot state={state} />
              <SessionKindIcon session={live ?? { kind: 'agent_tui', agent: s.agent }} />
              <span className={cn('min-w-0 flex-1 truncate', live && sessionUnseen(live) && 'font-semibold')}>{live?.title ?? s.title}</span>
              {state.label && <span className="shrink-0 text-meta text-muted-foreground">{state.label}</span>}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/**
 * True when the latest attempt is merged and the ticket is still open, or Done with the worktree still there (kept,
 * or a finish cut short, PDT-97): offer to finish it.
 */
export const showMerged = (t: Pick<Ticket, 'stage' | 'attempt'>) =>
  t.attempt?.state === 'merged' && (!isClosed(t.stage) || (t.stage === 'done' && t.attempt.worktreeId !== null));

/** "Merged into main." with the one-click finish: Done, then the worktree removed (only the latter once Done). */
export function MergedBanner({ ticket: t, className }: { ticket: Ticket; className?: string }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [finish, pending] = useAction(() => finishMerged(confirm, qc, t));
  if (!showMerged(t)) return null;
  const label = t.stage === 'done' ? 'Remove the worktree' : 'Move to Done and remove the worktree';
  return (
    <Banner
      tone="accent"
      role="status"
      className={className}
      actions={
        // The full label needs ~24rem; in a narrower banner (phone peek, thin pane) it would be clipped, so say less.
        <Button
          size="sm"
          compact
          variant="primary"
          pending={pending}
          aria-label={label}
          onClick={() => void finish()}
        >
          <span className="hidden @sm:inline">{label}</span>
          <span className="@sm:hidden">{t.stage === 'done' ? 'Remove' : 'Move to Done'}</span>
        </Button>
      }
    >
      <GitMerge aria-hidden className="mr-1 inline size-3.5 align-[-2px]" />
      Merged into {t.attempt!.baseBranch}.
    </Banner>
  );
}
