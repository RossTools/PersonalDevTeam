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
import { AgentDot, agentState, itemState } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { useAction } from '../common/useAction';
import { abandonAttempt, finishMerged, useOpenSession } from './actions';
import { isClosed } from './model';
import { useAllSessions } from './queries';
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

/** The latest attempt: branch, state, its sessions (click opens the tab), and Abandon… while it is active. */
export function AttemptSection({ ticket: t, compact = false }: { ticket: Ticket; compact?: boolean }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const openSession = useOpenSession();
  const all = useAllSessions();
  const [abandon, abandoning] = useAction(() => abandonAttempt(confirm, qc, t, t.attempt!));
  const a = t.attempt;
  if (!a) return null;
  const sessions = compact ? t.sessions.filter((s) => s.attemptId === a.id) : t.sessions;
  return (
    <OverviewSection
      title="Attempt"
      actions={a.state === 'active' && (
        <Button variant="ghost" size="sm" compact pending={abandoning} onClick={() => void abandon()}>Abandon…</Button>
      )}
    >
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <span className="min-w-0 flex-1 truncate font-mono" title={a.branch}>{a.branch}</span>
        <AttemptBadge state={a.state} />
      </div>
      {!compact && <p className="text-meta text-muted-foreground">Base {a.baseBranch}{t.attempts.length > 1 && ` · attempt ${t.attempts.length}`}</p>}
      {sessions.length > 0 ? (
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
                  <span className={cn('min-w-0 flex-1 truncate', live?.unseen && 'font-semibold')}>{live?.title ?? s.title}</span>
                  {state.label && <span className="shrink-0 text-meta text-muted-foreground">{state.label}</span>}
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="text-xs text-muted-foreground">No sessions.</p>
      )}
    </OverviewSection>
  );
}

/** True when the latest attempt is merged and the ticket is still open: offer to finish it. */
export const showMerged = (t: Pick<Ticket, 'stage' | 'attempt'>) => t.attempt?.state === 'merged' && !isClosed(t.stage);

/** "Merged into main." with the one-click finish: Done, then the worktree removed. */
export function MergedBanner({ ticket: t, className }: { ticket: Ticket; className?: string }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [finish, pending] = useAction(() => finishMerged(confirm, qc, t));
  if (!showMerged(t)) return null;
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
          aria-label="Move to Done and remove the worktree"
          onClick={() => void finish()}
        >
          <span className="hidden @sm:inline">Move to Done and remove the worktree</span>
          <span className="@sm:hidden">Move to Done</span>
        </Button>
      }
    >
      <GitMerge aria-hidden className="mr-1 inline size-3.5 align-[-2px]" />
      Merged into {t.attempt!.baseBranch}.
    </Banner>
  );
}
