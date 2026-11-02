// Make ticket… from a scratch chat (structured agents spec §10): the agent drafts a title and description in a hidden
// side request (it forks the conversation and changes nothing), then the new-ticket form opens with them. "Start from
// this chat" (a chat in its own worktree) makes the chat the ticket's attempt; nothing is sent to the agent.
import { Suspense, useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Dialog, Spinner } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketDraft } from '../api/gen/TicketDraft';
import { useAgents } from '../common/agents';
import { actionError } from '../common/useAction';
import { NewTicketDialog } from '../shell/screens';
import { invalidateTickets } from './queries';
import { invalidateWorktrees } from '../common/worktrees';

export interface MakeTicketDialogProps {
  session: Session;
  /** The chat is in a worktree off the base branch, so the ticket can be started from it. */
  canStartHere: boolean;
  onClose: () => void;
  onCreated?: (t: Ticket) => void;
}

type Phase = { step: 'drafting' } | { step: 'failed'; error: string } | { step: 'form'; draft: TicketDraft };

export function MakeTicketDialog({ session, canStartHere, onClose, onCreated }: MakeTicketDialogProps) {
  const qc = useQueryClient();
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const [phase, setPhase] = useState<Phase>({ step: 'drafting' });
  const [attempt, setAttempt] = useState(0);
  const agents = useAgents();
  const agentName = session.agent ? agents.label(session.agent) : 'Claude';

  useEffect(() => {
    // Asked once per try; closing the dialog stops the request.
    const ac = new AbortController();
    setPhase({ step: 'drafting' });
    api.draftTicket(session.id, ac.signal).then(
      (draft) => setPhase({ step: 'form', draft }),
      (e) => {
        if (!ac.signal.aborted) setPhase({ step: 'failed', error: actionError(e) });
      },
    );
    return () => ac.abort();
  }, [session.id, attempt]);

  if (phase.step === 'form' && projects.data) {
    const startHere = async (t: Ticket) => {
      await api.startTicket(t.id, { agent: session.agent ?? 'claude', fromSession: session.id, mode: 'force' });
      invalidateTickets(qc);
      invalidateWorktrees(qc, t.projectId);
      void qc.invalidateQueries({ queryKey: ['sessions'] });
      void qc.invalidateQueries({ queryKey: ['workspaces'] });
    };
    return (
      <Suspense fallback={null}><NewTicketDialog
        heading="Make ticket"
        projects={projects.data}
        projectId={session.projectId}
        title={phase.draft.title}
        description={phase.draft.description}
        then={canStartHere ? {
          label: 'Start from this chat',
          hint: 'Moves it to In progress with this chat and its branch as the attempt. Nothing is sent to the agent.',
          run: startHere,
        } : undefined}
        onClose={onClose}
        onCreated={onCreated}
      /></Suspense>
    );
  }

  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title="Make ticket"
      description={`${agentName} drafts a title and description from this chat. The chat itself is left as it is.`}
      size="md"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          {phase.step === 'failed' && (
            <>
              <Button variant="secondary" onClick={() => setPhase({ step: 'form', draft: { title: '', description: '' } })}>
                Write it myself
              </Button>
              <Button variant="primary" onClick={() => setAttempt((n) => n + 1)}>Try again</Button>
            </>
          )}
        </>
      }
    >
      {phase.step === 'failed' ? (
        <Alert tone="error" title="No draft">{phase.error}</Alert>
      ) : (
        <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground">
          <Spinner label="Drafting" className="size-4" /> Drafting a ticket from this chat…
        </p>
      )}
    </Dialog>
  );
}
