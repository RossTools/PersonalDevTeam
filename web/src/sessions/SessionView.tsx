import { memo, useEffect, useMemo, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, MenuButton, SegmentedControl, toast, useConfirm, ViewHeader, type MenuEntry } from '@ui';
import { api, ApiError } from '../api/client';
import type { Session } from '../api/gen/Session';
import type { SessionMode } from '../api/gen/SessionMode';
import { agentState, AgentDot, AgentLabel } from '../common/AgentState';
import { BranchChip, ProjectKey } from '../common/BranchChip';
import { copyToClipboard } from '../common/clipboard';
import { KIND_LABEL, SessionKindIcon } from '../common/SessionKindIcon';
import { isAcp } from '../common/agents';
import { actionError, useAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { ConversationPane } from '../conversation/ConversationView';
import type { EventsSocketLike } from '../conversation/eventsSocket';
import { useConversation, type ConversationData } from '../conversation/useConversation';
import { branchUrl, repoUrl } from '../projects/forgeLinks';
import type { ForgeLinks } from '../api/gen/ForgeLinks';
import { TerminalView, type TerminalHandle } from '../terminal/TerminalView';
import { FONT_SIZES, setTerminalFontSize, useTerminalFontSize } from '../terminal/fontSize';
import { MakeTicketDialog } from '../tickets/MakeTicketDialog';
import { EndedBar } from './EndedBar';
import { INITIAL_SIZE } from './status';
import { useViewTracking } from './useViewTracking';
import { usePolledAt } from '../common/usePolledAt';
import { worktreesQuery } from '../common/worktrees';
import { useOpenChanges, openExternal } from '../common/open';
import { ChangesButton } from '../review/ChangesButton';
import { DocumentsBesideLayout, useOpenBesidePicker } from '../documents/BesidePanel';
import { useProjects } from '../projects/queries';
import { invalidateSessions, sessionKeys } from './queries';

export interface SessionViewProps {
  session: Session;
  onRemoved: () => void;
  /** The session's branch on its forge, when known. */
  branchLink?: string | null;
  /** The worktree's branch; looked up from the project's worktrees when left out. */
  branch?: string | null;
  /** The project's repository on its forge; read from the projects list when left out. */
  forge?: ForgeLinks | null;
  /** The project's base branch (the branch chip's "Open PR"); read from the projects list when left out. */
  baseBranch?: string;
  /** The project's key, shown for the main checkout; read from the projects list when left out. */
  projectKey?: string;
  /** Tests: a fake events WebSocket for a conversation. */
  socketFactory?: (url: string) => EventsSocketLike;
}

/**
 * A session: its header over its terminal, or over its conversation (one events socket shared by both). Memoized: the
 * shell re-renders on its other polls, which change nothing here (PDT-454).
 */
export const SessionView = memo(function SessionView(props: SessionViewProps) {
  return props.session.mode === 'conversation'
    ? <ConversationSession key={props.session.id} {...props} />
    : <SessionBody {...props} conv={null} />;
});

function ConversationSession(props: SessionViewProps) {
  const conv = useConversation(props.session.id, props.socketFactory);
  return <SessionBody {...props} conv={conv} />;
}

type ConvView = 'conversation' | 'terminal';
const viewKey = (id: string) => `pdt.sessionView.${id}`;

/** The Conversation | Terminal choice for one session, per browser (it outlives tab switches and polls). */
function useStoredView(sessionId: string): [ConvView, (v: ConvView) => void] {
  const [view, setView] = useState<ConvView>(() => {
    try {
      return localStorage.getItem(viewKey(sessionId)) === 'terminal' ? 'terminal' : 'conversation';
    } catch {
      return 'conversation';
    }
  });
  const set = (v: ConvView) => {
    setView(v);
    try {
      if (v === 'terminal') localStorage.setItem(viewKey(sessionId), v);
      else localStorage.removeItem(viewKey(sessionId));
    } catch {
      // Storage blocked: the choice lasts while the view is open.
    }
  };
  return [view, set];
}

function SessionBody({ session, onRemoved, branchLink, branch, forge, baseBranch, projectKey, conv }: SessionViewProps & {
  conv: ConversationData | null;
}) {
  useViewTracking(session.id, session.unseen ? session.attentionAt ?? 0 : null);
  // The terminal's reattach check runs on every poll of the project's sessions (they hold this session).
  const polledAt = usePolledAt(sessionKeys.project(session.projectId));
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [composing, setComposing] = useState(false);

  // Shared with the workspaces' cached queries, so these are usually free.
  const needProject = forge === undefined || baseBranch === undefined || (projectKey === undefined && !session.worktreeId);
  const projects = useProjects({ enabled: needProject });
  const worktrees = useQuery({
    ...worktreesQuery(session.projectId),
    enabled: !!session.worktreeId && branch === undefined,
  });
  const project = projects.data?.find((p) => p.id === session.projectId);
  const theForge = forge !== undefined ? forge : project?.forge ?? null;
  const theBase = baseBranch ?? project?.baseBranch;
  const theKey = projectKey ?? project?.key;
  const theBranch = !session.worktreeId
    ? null
    : branch !== undefined ? branch : worktrees.data?.find((w) => w.id === session.worktreeId)?.branch ?? null;
  const forgeLink = branchLink
    ?? (theForge ? (theBranch ? branchUrl(theForge, theBranch) : !session.worktreeId ? repoUrl(theForge) : null) : null);

  const refresh = () => invalidateSessions(qc, session.projectId);
  const [kill, killing] = useAction(async () => { await api.killSession(session.id); void refresh(); });
  const [restart, restarting] = useAction(async () => { await api.restartSession(session.id, INITIAL_SIZE); void refresh(); });
  const [remove, removing] = useAction(async () => {
    await api.deleteSession(session.id);
    void refresh();
    onRemoved();
  });
  const running = session.status === 'running';
  const ended = session.status === 'finished' || session.status === 'interrupted';
  // One action at a time: the daemon serializes them anyway, and a second click would only fail.
  const busy = killing || restarting || removing;
  const state = agentState({ kind: session.kind, status: session.status, activity: session.activity, background: session.background, unseen: session.unseen });
  const isAgent = session.kind === 'agent_tui';
  // Conversations have no terminal: sending a message reopens them, so no Compose, Restart or ended bar.
  const conversation = session.mode === 'conversation';
  const restartLabel = session.status === 'interrupted' ? 'Resume' : 'Restart';
  // A scratch chat (a conversation with no ticket) can become a ticket.
  const [makingTicket, setMakingTicket] = useState(false);
  const openChanges = useOpenChanges();
  const scratch = conversation && isAgent && !session.attemptId;

  // Codex conversations: the agent's own TUI can be open beside the conversation, live on the same thread.
  const switchable = conversation && session.agent === 'codex' && conv !== null;
  const terminalOpen = conv?.state?.terminalOpen ?? false;
  // Codex can only resume a thread after its first turn.
  const hasTurn = useMemo(() => conv?.rows.some((r) => r.event.kind === 'turn') ?? false, [conv?.rows]);
  const [view, setView] = useStoredView(session.id);
  // Between the open request succeeding and the state saying so: show the terminal already.
  const [opening, setOpening] = useState(false);
  useEffect(() => {
    if (terminalOpen) setOpening(false);
  }, [terminalOpen]);
  const showTerminal = switchable && view === 'terminal' && (terminalOpen || opening);
  const [openTerminal, openingTerminal] = useAction(async () => {
    if (!terminalOpen) {
      try {
        await api.openSessionTerminal(session.id, INITIAL_SIZE);
      } catch (err) {
        if (err instanceof ApiError && err.code === 'no_turn_yet') throw new Error('Send a first message, then open the terminal.');
        throw err;
      }
      setOpening(true);
    }
    setView('terminal');
  });
  const [closeTerminal, closingTerminal] = useAction(async () => {
    await api.closeSessionTerminal(session.id);
    setOpening(false);
    setView('conversation');
  });
  const pickView = (v: ConvView) => {
    if (v === 'terminal') void openTerminal();
    else setView('conversation');
  };

  // Claude and Devin: the session is either the conversation or the agent's own TUI, one at a time (Devin
  // locks a session to one process).
  const oneAtATime = isAgent && (session.agent === 'claude' || session.agent === 'devin');
  const agentName = KIND_LABEL[session.agent === 'devin' ? 'devin' : 'claude'];
  const [switching, setSwitching] = useState<SessionMode | null>(null);
  const switchMode = async (mode: SessionMode) => {
    if (mode === session.mode || switching) return;
    if (mode === 'conversation' && running && session.activity === 'working') {
      const { ok } = await confirm({
        title: 'Back to the conversation?',
        body: `${agentName} is working in the terminal. Switching closes the terminal, which stops this turn.`,
        action: 'Close terminal and switch',
      });
      if (!ok) return;
    }
    setSwitching(mode);
    try {
      const body = { mode, ...INITIAL_SIZE };
      try {
        await api.switchSessionMode(session.id, body);
      } catch (err) {
        if (!(err instanceof ApiError && err.code === 'turn_running')) throw err;
        const { ok } = await confirm({
          title: 'Interrupt and open the terminal?',
          body: `${agentName} is working on a turn. Opening the terminal stops it; you can carry on from there.`,
          action: 'Interrupt and open',
        });
        if (!ok) return;
        await api.switchSessionMode(session.id, { ...body, interrupt: true });
      }
      await refresh();
    } catch (err) {
      toast.error(actionError(err));
    } finally {
      setSwitching(null);
    }
  };

  // Stopping is reversible (Restart resumes), so it asks only when it would cut off an agent mid-turn.
  const askStop = async () => {
    if (session.activity === 'working') {
      const { ok } = await confirm({
        title: `Stop ${session.title}?`,
        body: conversation
          ? 'The agent is working; stopping it interrupts the current turn. Send a message to start it again.'
          : 'The agent is working; stopping it interrupts the current turn. Restart resumes its conversation.',
        action: 'Stop agent',
        danger: true,
      });
      if (!ok) return;
    }
    await kill();
  };
  // A modal every time: removing cannot be undone, and a running session is stopped first.
  const askRemove = async () => {
    const lost = conversation
      ? 'Its conversation history is deleted. Files in its folder stay.'
      : isAgent
      ? 'Its terminal history is deleted and it can no longer be restarted or resumed here. Files in its folder stay.'
      : 'Its terminal history is deleted and it can no longer be restarted. Files in its folder stay.';
    const { ok } = await confirm({
      title: `Remove ${session.title}?`,
      body: !running
        ? lost
        : session.activity === 'working'
          ? `The agent is working; removing it stops it mid-turn. ${lost}`
          : `It is still running and will be stopped. ${lost}`,
      action: running ? 'Stop and remove' : 'Remove session',
      danger: true,
    });
    if (ok) await remove();
  };

  // Documents beside the conversation (PDT-317).
  const besidePicker = useOpenBesidePicker(session.id, conversation);
  // On a phone, Compose sits with the terminal's other actions (terminalEntries).
  const entries = (compose = true): MenuEntry[] => [
    // Devin and ACP agents cannot draft on the side without touching the conversation (the server says
    // draft_unsupported).
    ...(scratch && session.agent !== 'devin' && !isAcp(session.agent) ? [{ label: 'Make ticket…', onSelect: () => setMakingTicket(true) }, { separator: true } as const] : []),
    ...(conversation ? [besidePicker.entry, { separator: true } as const] : []),
    ...(conversation
      ? [
          ...(terminalOpen ? [{ label: 'Close terminal', onSelect: () => void closeTerminal(), disabled: closingTerminal }] : []),
          ...(running ? [{ label: 'Stop', onSelect: () => void askStop(), disabled: busy }] : []),
          ...(terminalOpen || running ? [{ separator: true } as const] : []),
        ]
      : [
          ...(running
            ? [
                ...(compose ? [{ label: 'Compose', onSelect: () => setComposing((c) => !c) }] : []),
                { label: 'Stop', onSelect: () => void askStop(), disabled: busy },
              ]
            : []),
          { label: restartLabel, onSelect: () => void restart(), disabled: busy },
          { separator: true } as const,
        ]),
    { label: 'Copy path', onSelect: () => void copyToClipboard(session.cwd, 'Path copied') },
    ...(forgeLink ? [{ label: `Open on ${theForge?.name ?? 'the forge'}`, onSelect: () => openExternal(forgeLink) }] : []),
    { separator: true },
    { label: 'Remove…', onSelect: () => void askRemove(), disabled: busy, danger: true },
  ];

  // Phones (PDT-114): the header is one line (state, title, branch) and every action is under its ⋯.
  const phone = useMediaQuery(PHONE_QUERY);
  // The terminal's own actions (PDT-115): Paste and Font size, beside Compose.
  const terminal = useRef<TerminalHandle>(null);
  const fontSize = useTerminalFontSize();
  const terminalShown = showTerminal || !conv;
  const terminalEntries = (): MenuEntry[] => terminalShown
    ? [
        ...(running ? [{ label: 'Paste', onSelect: () => terminal.current?.paste() }] : []),
        ...(running && !conversation ? [{ label: 'Compose', onSelect: () => setComposing((c) => !c) }] : []),
        {
          submenu: `Font size (${fontSize})`,
          entries: FONT_SIZES.map((n) => ({ label: n === fontSize ? `${n} ✓` : String(n), onSelect: () => setTerminalFontSize(n) })),
        },
        { separator: true } as const,
      ]
    : [];
  const phoneEntries = (): MenuEntry[] => [
    ...(session.worktreeId ? [{ label: 'Changes', onSelect: () => openChanges({ id: session.worktreeId!, projectId: session.projectId }, { here: true }) }] : []),
    ...(oneAtATime
      ? [{
          label: session.mode === 'conversation' ? 'Show as terminal' : 'Show as conversation',
          onSelect: () => void switchMode(session.mode === 'conversation' ? 'terminal' : 'conversation'),
          disabled: switching !== null || busy,
        }]
      : []),
    ...(switchable
      ? [{
          label: showTerminal ? 'Show as conversation' : 'Show as terminal',
          onSelect: () => pickView(showTerminal ? 'conversation' : 'terminal'),
          disabled: openingTerminal || (!showTerminal && !hasTurn && !terminalOpen),
        }]
      : []),
    ...(session.worktreeId || oneAtATime || switchable ? [{ separator: true } as const] : []),
    ...terminalEntries(),
    ...entries(false),
  ];

  return (
    <div className="flex flex-1 flex-col min-h-0">
      {besidePicker.dialog}
      {makingTicket && (
        <MakeTicketDialog
          session={session}
          canStartHere={!!theBranch && !!theBase && theBranch !== theBase}
          onClose={() => setMakingTicket(false)}
          onCreated={(t) => toast.success(`Created ${t.key}`)}
        />
      )}
      {phone ? (
        <ViewHeader
          aria-label="Session"
          className="flex-nowrap"
          leading={
            <span className="flex items-center gap-1.5">
              <AgentDot state={state} />
              <AgentLabel state={state} className="shrink-0" />
            </span>
          }
          title={session.title}
          titleTooltip={`${session.title} · ${session.cwd}`}
          meta={theBranch ? <BranchChip branch={theBranch} forge={theForge} base={theBase} className="min-w-0" /> : undefined}
          actions={<MenuButton label="Session actions" placement="bottom end" entries={phoneEntries}/>}
        />
      ) : (
      <ViewHeader
        leading={
          <span className="flex items-center gap-1.5">
            <AgentDot state={state} />
            <SessionKindIcon session={session} />
          </span>
        }
        title={session.title}
        titleTooltip={`${session.title} · ${session.cwd}`}
        meta={
          <>
            {theBranch ? (
              <BranchChip
                branch={theBranch}
                forge={theForge}
                base={theBase}
                {...(session.worktreeId ? { onReview: () => openChanges({ id: session.worktreeId!, projectId: session.projectId }, { here: true }) } : {})}
                className="min-w-0"
              />
            ) : (
              !session.worktreeId && theKey && <ProjectKey projectKey={theKey} />
            )}
            <AgentLabel state={state} className="shrink-0" />
          </>
        }
        actions={
          <>
            {session.worktreeId && <ChangesButton worktreeId={session.worktreeId} projectId={session.projectId} />}
            {oneAtATime && (
              <SegmentedControl<SessionMode>
                aria-label="Show as"
                size="sm"
                value={switching ?? session.mode}
                onChange={(v) => void switchMode(v)}
                disabled={switching !== null || busy}
                options={[
                  { value: 'conversation', label: 'Conversation' },
                  { value: 'terminal', label: 'Terminal' },
                ]}
              />
            )}
            {switchable && (
              // A disabled segment gets no hover of its own, so the wrapper carries the explanation.
              <span className="flex" title={hasTurn || terminalOpen ? undefined : 'Send a first message, then open the terminal'}>
                  <SegmentedControl<ConvView>
                    aria-label="Show as"
                    size="sm"
                    value={showTerminal ? 'terminal' : 'conversation'}
                    onChange={pickView}
                    disabled={openingTerminal}
                    options={[
                      { value: 'conversation', label: 'Conversation' },
                      { value: 'terminal', label: 'Terminal', disabled: !hasTurn && !terminalOpen },
                    ]}
                  />
              </span>
            )}
            {ended && !conversation && (
              <Button variant="primary" size="sm" disabled={busy} pending={restarting} onClick={() => void restart()}>{restartLabel}</Button>
            )}
            <MenuButton label="Session actions" size="sm" placement="bottom end" entries={() => entries()} />
          </>
        }
      />
      )}
      {showTerminal ? (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <TerminalView key={`${session.id}:tui`} handle={terminal} sessionId={session.id} sessionStatus={session.status} polledAt={polledAt} />
        </div>
      ) : conv ? (
        <DocumentsBesideLayout sessionId={session.id} phone={phone}>
          <ConversationPane
            session={session}
            conv={conv}
            banner={switchable && terminalOpen ? (
              <p className="flex items-center justify-center gap-2 px-4 pb-1 text-meta text-muted-foreground" role="status">
                Also open in the terminal
                <Button variant="ghost" size="sm" onClick={() => pickView('terminal')}>Show</Button>
              </p>
            ) : undefined}
          />
        </DocumentsBesideLayout>
      ) : (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <TerminalView
            key={session.id}
            handle={terminal}
            sessionId={session.id}
            sessionStatus={session.status}
            polledAt={polledAt}
            composing={composing && running}
            onComposeClose={() => setComposing(false)}
            dimmed={ended}
          />
          {ended && (
            <EndedBar
              status={session.status === 'interrupted' ? 'interrupted' : 'finished'}
              agent={isAgent}
              onRestart={() => void restart()}
              onRemove={() => void askRemove()}
              restarting={restarting}
              removing={removing}
              disabled={busy}
            />
          )}
        </div>
      )}
    </div>
  );
}
