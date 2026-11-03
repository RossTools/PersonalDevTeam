import { TriangleAlert } from 'lucide-react';
import { Button } from '@ui';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { requestAnchor } from './MessageList';
import { shortTool } from './tools';

type Request = Extract<AgentEvent, { kind: 'request' }>;

/** The conversation's open requests, oldest first. */
function openRequests(rows: ConversationEvent[]): Request[] {
  return rows.flatMap((r) => (r.event.kind === 'request' && r.event.state === 'open' ? [r.event] : []));
}

/** "wants create_ticket", "has a question": what the request asks for, after the ⚠. */
function requestLine(ev: Request): string {
  switch (ev.requestKind) {
    case 'question':
      return 'has a question';
    case 'plan':
      return 'wants its plan approved';
    default:
      return `wants ${shortTool(ev.tool)}`;
  }
}

/**
 * Over the composer while the agent waits on a request (PDT-113), in every conversation: "⚠ wants create_ticket ·
 * Review", where Review scrolls to the request's card and focuses it.
 */
export function PinnedApprovalBar({ rows }: { rows: ConversationEvent[] }) {
  const open = openRequests(rows);
  const first = open[0];
  if (!first) return null;
  const review = () => {
    const card = document.getElementById(requestAnchor(first.id));
    if (!card) return;
    card.scrollIntoView?.({ block: 'center' });
    card.focus({ preventScroll: true });
  };
  return (
    <div role="region" aria-label="Open request" className="mx-auto flex w-full max-w-3xl items-center gap-2 px-4 pb-1">
      <div className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md border border-waiting bg-card py-0.5 pr-0.5 pl-2 text-xs">
        <TriangleAlert aria-hidden className="size-3.5 shrink-0 text-waiting" />
        <span className="min-w-0 flex-1 truncate">
          {requestLine(first)}
          {open.length > 1 && <span className="text-muted-foreground"> · {open.length - 1} more</span>}
        </span>
        <Button variant="ghost" size="sm" onClick={review} className="shrink-0 pointer-coarse:min-h-control-touch">Review</Button>
      </div>
    </div>
  );
}
