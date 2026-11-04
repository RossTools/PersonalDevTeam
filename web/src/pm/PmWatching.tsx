// What a PM conversation is watching (PDT-555): an eye with a count in its header that opens the list of its watches,
// each with what it hears about and Stop watching. The list comes from every watch (one request, shared with the
// board's and the PM list's eyes), so it updates on the next poll when a watch fires or ends.
import { useQueryClient } from '@tanstack/react-query';
import { Eye } from 'lucide-react';
import { Badge, Button, Link, Popover, RelativeTime, useConfirm } from '@ui';
import type { TicketStage } from '../api/gen/TicketStage';
import type { WatchInfo } from '../api/gen/WatchInfo';
import type { WorkflowRunStatus } from '../api/gen/WorkflowRunStatus';
import { STAGE_LABEL } from '../common/StageStepper';
import { TicketPill } from '../common/TicketPill';
import { plural } from '../common/text';
import { runAction } from '../common/useAction';
import { stopWatch, useAllWatches } from '../common/Watch';
import { watchEventsText } from '../conversation/pdtTools';
import { RUN_STATUS } from '../workflows/runs';

/** A conversation's watches, oldest first (empty while loading). */
export function useSessionWatches(sessionId: string): WatchInfo[] {
  return useAllWatches().data?.filter((w) => w.sessionId === sessionId) ?? [];
}

/** The watched ticket or run: its pill and title, or its workflow's name, with its stage or status. */
function Target({ w }: { w: WatchInfo }) {
  if (w.target === 'ticket') {
    const stage = w.targetState as TicketStage | null;
    return (
      <span className="flex min-w-0 items-center gap-1.5">
        <TicketPill ticketKey={w.targetLabel} />
        <span className="min-w-0 flex-1 truncate" title={w.targetTitle ?? undefined}>{w.targetTitle}</span>
        {stage && STAGE_LABEL[stage] && <Badge>{STAGE_LABEL[stage]}</Badge>}
      </span>
    );
  }
  const status = w.targetState ? RUN_STATUS[w.targetState as WorkflowRunStatus] : undefined;
  const href = w.workflowId ? `/workflows/${encodeURIComponent(w.workflowId)}/runs/${encodeURIComponent(w.targetId)}` : undefined;
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="min-w-0 flex-1 truncate">
        Run of {href ? <Link href={href} tone="quiet">{w.targetLabel}</Link> : w.targetLabel}
      </span>
      {status && <Badge tone={status.tone}>{status.text}</Badge>}
    </span>
  );
}

function WatchRow({ w, title }: { w: WatchInfo; title: string }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const stop = () => runAction(async () => {
    const { ok } = await confirm({
      title: `Stop watching ${w.target === 'ticket' ? w.targetLabel : `the run of ${w.targetLabel}`}?`,
      body: `“${title}” won't hear about it any more. The PM may be waiting on it.`,
      action: 'Stop watching',
    });
    if (ok) await stopWatch(qc, w.id);
  });
  return (
    <li className="flex flex-col gap-1 py-2 first:pt-0 last:pb-0">
      <Target w={w} />
      <span className="text-meta text-muted-foreground">
        Hears when {watchEventsText(w.events)} · {w.once ? 'until it finishes' : 'ongoing'} · since <RelativeTime time={w.createdAt} />
      </span>
      <Button variant="ghost" size="sm" className="self-start" onClick={() => void stop()}>Stop watching</Button>
    </li>
  );
}

/** "👁 3" in a PM conversation's header while it watches anything; it opens the list. Nothing otherwise. */
export function PmWatching({ sessionId, title }: { sessionId: string; title: string }) {
  const watches = useSessionWatches(sessionId);
  if (watches.length === 0) return null;
  const label = `Watching ${plural(watches.length, 'thing')}`;
  return (
    <Popover
      title="Watching"
      placement="bottom end"
      className="w-96"
      trigger={(
        <Button variant="ghost" size="sm" aria-label={label} title={label}>
          <Eye aria-hidden /> <span className="tabular-nums">{watches.length}</span>
        </Button>
      )}
    >
      <ul aria-label="Watches" className="flex flex-col divide-y divide-border text-sm">
        {watches.map((w) => <WatchRow key={w.id} w={w} title={title} />)}
      </ul>
    </Popover>
  );
}
