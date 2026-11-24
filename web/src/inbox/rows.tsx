// What an Inbox list row shows (PDT-686): its title on up to two lines, a second line that tells it from the others
// in its section, and small actions that grow to a finger's size on a touch screen.
import { useMemo } from 'react';
import type { AgentKind } from '../api/gen/AgentKind';
import type { InboxItem } from '../api/gen/InboxItem';
import { agentLabel, useAcpAgents } from '../common/agents';
import { useProjects } from '../projects/queries';
import { useAllSessions } from '../tickets/queries';
import { KIND_LABEL, shownKind } from './model';

/** What a row's second line names besides the item: its session and the project's key. */
export interface RowContext {
  /** The asking session's title. */
  sessionTitle?: string | null;
  /** The agent's name: "Claude". */
  agent?: string | null;
  projectKey?: string | null;
}

/**
 * What a row says under its title, so rows in one section tell apart: who asks (or which workflow), the agent, then
 * the ticket's title (its key is beside it) or else the session's title and the project's key. The kind's label only
 * when there is nothing else.
 */
export function context(i: InboxItem, c: RowContext = {}): string {
  const who = i.personaName ?? (typeof i.body.workflow === 'string' ? i.body.workflow : null);
  const sessionTitle = c.sessionTitle ?? (typeof i.body.sessionTitle === 'string' ? i.body.sessionTitle : null);
  const parts = i.ticketKey
    ? [who, c.agent, i.ticketTitle]
    : [who, c.agent, sessionTitle, c.projectKey];
  const shown = [...new Set(parts.filter((p): p is string => !!p))];
  return shown.length > 0 ? shown.join(' · ') : KIND_LABEL[shownKind(i)];
}

/** Each item's row context, from the sessions and projects the app already polls. */
export function useRowContext(): (i: InboxItem) => RowContext {
  const sessions = useAllSessions();
  const projects = useProjects().data;
  const acp = useAcpAgents();
  return useMemo(() => {
    const byId = new Map(sessions.map((s) => [s.id, s]));
    const keys = new Map((projects ?? []).map((p) => [p.id, p.key]));
    return (i: InboxItem) => {
      const session = i.sessionId ? byId.get(i.sessionId) ?? null : null;
      const agent = session?.agent ?? (typeof i.body.agent === 'string' ? i.body.agent : null);
      return {
        sessionTitle: session?.title ?? null,
        agent: agent ? agentLabel(agent as AgentKind, acp) : null,
        projectKey: i.projectId ? keys.get(i.projectId) ?? null : null,
      };
    };
  }, [sessions, projects, acp]);
}

/** A list row's title: up to two lines, so a long question still says what it asks. */
export function RowTitle({ children }: { children: string }) {
  return <span className="line-clamp-2 whitespace-normal break-words">{children}</span>;
}
