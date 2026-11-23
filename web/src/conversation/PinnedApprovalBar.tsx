import { useEffect, useState } from 'react';
import { TriangleAlert } from 'lucide-react';
import { Button } from '@ui';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { requestAnchor } from './MessageList';
import { shortTool, wantsVerb } from './tools';

type Request = Extract<AgentEvent, { kind: 'request' }>;

/** The conversation's open requests, oldest first. */
function openRequests(rows: ConversationEvent[]): Request[] {
  return rows.flatMap((r) => (r.event.kind === 'request' && r.event.state === 'open' ? [r.event] : []));
}

/** "Claude wants to run `npm test`", "Claude has a question": what the request asks for, after the ⚠. */
function RequestLine({ ev, agentName }: { ev: Request; agentName: string }) {
  switch (ev.requestKind) {
    case 'question':
      return <>{agentName} has a question</>;
    case 'plan':
      return <>{agentName} wants its plan approved</>;
    default: {
      const verb = wantsVerb(ev.tool);
      return (
        <>
          {agentName} wants {verb ? `to ${verb}` : shortTool(ev.tool)}
          {ev.title && <> <code className="rounded-sm bg-muted px-1 py-px font-mono text-meta">{ev.title}</code></>}
        </>
      );
    }
  }
}

/** Whether the element with `id` is on screen; false while it isn't there or can't be watched. */
function useOnScreen(id: string | undefined, rows: number): boolean {
  const [on, setOn] = useState(false);
  useEffect(() => {
    const el = id ? document.getElementById(id) : null;
    if (!el || typeof IntersectionObserver === 'undefined') {
      setOn(false);
      return;
    }
    const io = new IntersectionObserver((entries) => setOn(entries.some((e) => e.isIntersecting)));
    io.observe(el);
    return () => io.disconnect();
    // A card that comes in later (more rows) is watched then.
  }, [id, rows]);
  return on;
}

/**
 * Over the composer while the agent waits on a request whose card is out of view (PDT-113, PDT-685), in every
 * conversation: "⚠ Claude wants to run `npm test` · Review", where Review scrolls to the request's card and focuses it.
 */
export function PinnedApprovalBar({ rows, agentName }: { rows: ConversationEvent[]; agentName: string }) {
  const open = openRequests(rows);
  const first = open[0];
  const cardOnScreen = useOnScreen(first && requestAnchor(first.id), rows.length);
  if (!first || cardOnScreen) return null;
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
          <RequestLine ev={first} agentName={agentName} />
          {open.length > 1 && <span className="text-muted-foreground"> · {open.length - 1} more</span>}
        </span>
        <Button variant="ghost" size="sm" onClick={review} className="shrink-0 pointer-coarse:min-h-control-touch">Review</Button>
      </div>
    </div>
  );
}
