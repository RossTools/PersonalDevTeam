import { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, MenuButton, useConfirm, ViewHeader, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { agentState, AgentDot, AgentLabel } from '../common/AgentState';
import { BranchChip, ProjectKey } from '../common/BranchChip';
import { copyToClipboard } from '../common/clipboard';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { useAction } from '../common/useAction';
import { ConversationView } from '../conversation/ConversationView';
import { branchUrl, repoUrl } from '../projects/github';
import { TerminalView } from '../terminal/TerminalView';
import { EndedBar } from './EndedBar';
import { INITIAL_SIZE } from './status';
import { useViewTracking } from './useViewTracking';

export function SessionView({ session, polledAt, onRemoved, branchLink, branch, repo, baseBranch, projectKey }: {
  session: Session;
  polledAt?: number;
  onRemoved: () => void;
  /** The session's branch on GitHub, when known. */
  branchLink?: string | null;
  /** The worktree's branch; looked up from the project's worktrees when left out. */
  branch?: string | null;
  /** The project's `owner/repo` on GitHub; read from the projects list when left out. */
  repo?: string | null;
  /** The project's base branch (the branch chip's "Open PR"); read from the projects list when left out. */
  baseBranch?: string;
  /** The project's key, shown for the main checkout; read from the projects list when left out. */
  projectKey?: string;
}) {
  useViewTracking(session.id, polledAt);
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [composing, setComposing] = useState(false);

  // Shared with the workspaces' cached queries, so these are usually free.
  const needProject = repo === undefined || baseBranch === undefined || (projectKey === undefined && !session.worktreeId);
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects, enabled: needProject, staleTime: 30_000 });
  const worktrees = useQuery({
    queryKey: ['worktrees', session.projectId],
    queryFn: () => api.listWorktrees(session.projectId),
    enabled: !!session.worktreeId && branch === undefined,
    staleTime: 30_000,
  });
  const project = projects.data?.find((p) => p.id === session.projectId);
  const theRepo = repo !== undefined ? repo : project?.githubRepo ?? null;
  const theBase = baseBranch ?? project?.baseBranch;
  const theKey = projectKey ?? project?.key;
  const theBranch = !session.worktreeId
    ? null
    : branch !== undefined ? branch : worktrees.data?.find((w) => w.id === session.worktreeId)?.branch ?? null;
  const githubLink = branchLink
    ?? (theRepo ? (theBranch ? branchUrl(theRepo, theBranch) : !session.worktreeId ? repoUrl(theRepo) : null) : null);

  const refresh = () => qc.invalidateQueries({ queryKey: ['sessions', session.projectId] });
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
  const state = agentState({ kind: session.kind, status: session.status, activity: session.activity, unseen: session.unseen });
  const isAgent = session.kind === 'agent_tui';
  // Conversations have no terminal: sending a message reopens them, so no Compose, Restart or ended bar.
  const conversation = session.mode === 'conversation';
  const restartLabel = session.status === 'interrupted' ? 'Resume' : 'Restart';

  // Stopping is reversible (Restart resumes), so it asks only when it would cut off an agent mid-turn (spec §6.2).
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

  const entries = (): MenuEntry[] => [
    ...(conversation
      ? running ? [{ label: 'Stop', onSelect: () => void askStop(), disabled: busy }, { separator: true } as const] : []
      : [
          ...(running
            ? [
                { label: 'Compose', onSelect: () => setComposing((c) => !c) },
                { label: 'Stop', onSelect: () => void askStop(), disabled: busy },
              ]
            : []),
          { label: restartLabel, onSelect: () => void restart(), disabled: busy },
          { separator: true } as const,
        ]),
    { label: 'Copy path', onSelect: () => void copyToClipboard(session.cwd, 'Path copied') },
    ...(githubLink ? [{ label: 'Open on GitHub', onSelect: () => void window.open(githubLink, '_blank', 'noopener,noreferrer') }] : []),
    { separator: true },
    { label: 'Remove…', onSelect: () => void askRemove(), disabled: busy, danger: true },
  ];

  return (
    <div className="flex flex-1 flex-col min-h-0">
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
              <BranchChip branch={theBranch} repo={theRepo} base={theBase} className="min-w-0" />
            ) : (
              !session.worktreeId && theKey && <ProjectKey projectKey={theKey} />
            )}
            <AgentLabel state={state} className="shrink-0" />
          </>
        }
        actions={
          <>
            {ended && !conversation && (
              <Button variant="primary" size="sm" disabled={busy} pending={restarting} onClick={() => void restart()}>{restartLabel}</Button>
            )}
            <MenuButton label="Session actions" size="sm" placement="bottom end" entries={entries} />
          </>
        }
      />
      {conversation ? (
        <ConversationView key={session.id} session={session} />
      ) : (
        <div className="relative flex min-h-0 flex-1 flex-col">
          <TerminalView
            key={session.id}
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
