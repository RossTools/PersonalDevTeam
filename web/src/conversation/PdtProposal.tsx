// What a PDT tool waiting for the user's yes would create or change (PDT-382), for its approval card.
import type { ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Badge } from '@ui';
import { api } from '../api/client';
import { ChangeRow, Key, priorityLabel, stageLabel } from './PdtToolCard';
import { createdTicket, LINK_TEXT, pdtApprovalTitle, pdtTool, sentMessage, ticketEdit, ticketLink } from './pdtTools';
import { isRecord } from '../common/guards';
import { sessionKeys } from '../sessions/queries';

/** A description's first paragraph, cut to a couple of lines. */
export function shortText(markdown: string, max = 220): string {
  const first = markdown.trim().split(/\n\s*\n/)[0].replace(/^#+\s*/, '').replace(/\s+/g, ' ').trim();
  return first.length > max ? `${first.slice(0, max - 1).trimEnd()}…` : first;
}

function Box({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-1.5 rounded-md border border-border bg-surface-2 p-2.5">{children}</div>;
}

function NewTicket({ input }: { input: unknown }) {
  const t = createdTicket(input);
  const a = isRecord(input) ? input : {};
  const parent = typeof a.parent === 'string' && a.parent ? a.parent : null;
  const reason = typeof a.reason === 'string' && a.reason.trim() ? a.reason.trim() : null;
  const files = Array.isArray(a.files) ? a.files.length : 0;
  return (
    <Box>
      <div className="font-semibold">{t.title}</div>
      {t.description && <p className="text-xs text-muted-foreground">{shortText(t.description)}</p>}
      <div className="flex flex-wrap items-center gap-1">
        <Badge>{stageLabel(t.stage)}</Badge>
        {t.priority && t.priority !== 'none' && <Badge>{priorityLabel(t.priority)}</Badge>}
        {t.labels.map((l) => <Badge key={l} tone="accent">{l}</Badge>)}
        {parent && <span className="ml-1 flex items-center gap-1 text-xs text-muted-foreground">Part of <Key value={parent} /></span>}
        {files > 0 && <span className="ml-1 text-xs text-muted-foreground">{files === 1 ? 'A file attached' : `${files} files attached`}</span>}
      </div>
      {reason && <p className="text-xs"><span className="text-muted-foreground">Why: </span>{reason}</p>}
    </Box>
  );
}

function SessionName({ id }: { id: string | null }) {
  const s = useQuery({ queryKey: sessionKeys.one(id ?? ''), queryFn: () => api.getSession(id!), enabled: !!id, staleTime: 60_000, retry: false });
  return <span className="font-medium">{s.data?.title ?? 'a session'}</span>;
}

/**
 * A PDT tool's approval, wherever it is asked (the conversation's card, the Inbox): its question ("Create this
 * ticket?") and what it would create or change. Null for a tool without one.
 */
export function pdtApproval(tool: string, input: unknown): { title: string; preview: ReactNode } | null {
  const title = pdtApprovalTitle(tool, input);
  return title ? { title, preview: <PdtProposal tool={tool} input={input} /> } : null;
}

/** The approval card's preview for a PDT tool (`pdtApprovalTitle` names the tools that have one). */
export function PdtProposal({ tool, input }: { tool: string; input: unknown }) {
  const a = isRecord(input) ? input : {};
  switch (pdtTool(tool)) {
    case 'create_ticket':
      return <NewTicket input={input} />;
    case 'edit_ticket': {
      const e = ticketEdit(input);
      const description = typeof a.description === 'string' ? a.description : null;
      return (
        <Box>
          <div className="flex flex-col divide-y divide-border">
            {e.changes.filter((c) => c.field !== 'description').map((c) => <ChangeRow key={c.field} c={c} />)}
            {description !== null && (
              <div className="flex flex-col gap-1 py-2">
                <span className="text-xs text-muted-foreground">New description</span>
                <p className="text-xs">{shortText(description) || 'Empty'}</p>
              </div>
            )}
            {e.files > 0 && <div className="py-2 text-xs text-muted-foreground">Attaches {e.files === 1 ? 'a file' : `${e.files} files`}</div>}
          </div>
        </Box>
      );
    }
    case 'link_tickets':
    case 'unlink_tickets': {
      const l = ticketLink(input);
      return (
        <Box>
          <div className="flex flex-wrap items-center gap-2">
            <Key value={l.ticket} />
            <span>{LINK_TEXT[l.kind]}</span>
            <Key value={l.other} />
          </div>
        </Box>
      );
    }
    case 'send_to_session': {
      const m = sentMessage(input);
      return (
        <Box>
          <div className="text-xs text-muted-foreground">To <SessionName id={m.session} /></div>
          <blockquote className="border-l-2 border-border pl-3 whitespace-pre-wrap">{m.text}</blockquote>
        </Box>
      );
    }
    case 'workflow_run': {
      const name = typeof a.name === 'string' ? a.name : typeof a.id === 'string' ? a.id : 'A workflow';
      const given = isRecord(a.input) ? Object.entries(a.input) : [];
      return (
        <Box>
          <div className="font-semibold">{name}</div>
          {typeof a.project === 'string' && <div className="text-xs text-muted-foreground">In {a.project}</div>}
          {given.length > 0 && (
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
              {given.map(([k, v]) => (
                <div key={k} className="contents">
                  <dt className="text-muted-foreground">{k}</dt>
                  <dd className="min-w-0 break-words">{typeof v === 'string' ? v : JSON.stringify(v)}</dd>
                </div>
              ))}
            </dl>
          )}
        </Box>
      );
    }
    case 'stop_ticket': {
      const stage = typeof a.stage === 'string' ? stageLabel(a.stage) : 'To do';
      return (
        <Box>
          <div className="flex flex-wrap items-center gap-1.5">
            <Key value={typeof a.ticket === 'string' ? a.ticket : null} />
            <span>stops and goes back to {stage}</span>
          </div>
          {a.removeWorktree === true && (
            <p className="text-xs text-failed">
              Its worktree is removed, uncommitted changes too{a.deleteBranch === true ? ', and its branch deleted' : ''}.
            </p>
          )}
        </Box>
      );
    }
    default:
      return null;
  }
}
