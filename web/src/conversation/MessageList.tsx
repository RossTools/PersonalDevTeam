import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowDown } from 'lucide-react';
import { Alert, ApprovalCard, Button, MessageBubble, Spinner, ToolCallRow, ToolOutput, type ApprovalAnswer } from '@ui';
import { cn } from '@ui/lib/utils';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { PendingMessage } from '../api/gen/PendingMessage';
import { Markdown } from '../common/Markdown';
import { buildBlocks, currentTool, turnStartedAt, type Block, type RowNode } from './blocks';
import type { Streams } from './model';
import { approvalTitle, clock, duration, inputText, planOf, questionsOf, summarizeTools, toolVerb } from './tools';

type Ev<K extends AgentEvent['kind']> = Extract<AgentEvent, { kind: K }>;

export interface MessageListProps {
  rows: ConversationEvent[];
  provisional: { itemId: string; markdown: string }[];
  streams: Streams;
  turnRunning: boolean;
  pending: PendingMessage[];
  /** Answers an open request (`request` event `id`). */
  onAnswer: (requestId: string, answer: ApprovalAnswer) => Promise<unknown>;
  /** Shown when there is nothing yet. */
  empty?: ReactNode;
  className?: string;
}

/** Offscreen rows skip layout and paint, so a long history stays cheap without a virtualiser. */
const ROW = '[content-visibility:auto] [contain-intrinsic-size:auto_3rem]';
/** How close to the bottom (px) still counts as "at the bottom". */
const STICK_PX = 48;

/** The time, updated every second while `on`. */
function useTicker(on: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [on]);
  return now;
}

function WorkingRow({ rows }: { rows: ConversationEvent[] }) {
  const now = useTicker(true);
  const tool = currentTool(rows);
  const since = tool?.ts ?? turnStartedAt(rows);
  const ev = tool?.event.kind === 'tool_call' ? tool.event : null;
  return (
    <div className="flex items-center gap-2 px-1 text-xs text-muted-foreground" data-testid="working">
      <Spinner decorative className="size-3.5" />
      <span className="flex min-w-0 items-baseline gap-1.5">
        {ev ? (
          <>
            <span className="shrink-0">{ev.tool === 'Bash' ? 'Running' : toolVerb(ev.tool)}</span>
            {ev.title && <code className="min-w-0 truncate rounded-sm bg-muted px-1 py-px font-mono text-meta text-foreground">{ev.title}</code>}
            <span aria-hidden>…</span>
          </>
        ) : (
          'Working…'
        )}
      </span>
      {since !== null && <span className="shrink-0 tabular-nums">{clock(now - since)}</span>}
    </div>
  );
}

function ToolCallNode({ node, streams }: { node: RowNode; streams: Streams }) {
  const ev = node.row.event as Ev<'tool_call'>;
  const output = (node.row.itemId && streams[node.row.itemId]?.output) || ev.output;
  const input = inputText(ev.tool, ev.input);
  const meta = ev.exitCode != null && ev.exitCode !== 0 ? `exit ${ev.exitCode}` : undefined;
  return (
    <ToolCallRow
      verb={toolVerb(ev.tool)}
      subject={ev.title || undefined}
      status={ev.status}
      meta={meta}
      details={
        input || output || ev.diff ? (
          <>
            {input && <ToolOutput label="Input">{input}</ToolOutput>}
            {ev.diff && <ToolOutput label="Diff">{ev.diff}</ToolOutput>}
            {output && <ToolOutput label="Output" tone={ev.status === 'failed' ? 'error' : 'default'}>{output}</ToolOutput>}
          </>
        ) : undefined
      }
    >
      {node.children.length > 0 ? node.children.map((c) => <NodeView key={c.row.itemId ?? c.row.seq} node={c} streams={streams} />) : undefined}
    </ToolCallRow>
  );
}

function resolvedText(ev: Ev<'request'>): string {
  if (ev.state === 'expired') return 'Expired';
  const a = ev.answer;
  if (!a) return 'Answered';
  switch (a.type) {
    case 'approve':
      return a.always ? 'Always allowed in this session' : 'Approved';
    case 'deny':
      return a.message ? `Denied: ${a.message}` : 'Denied';
    case 'question':
      return `Answered: ${Object.values(a.answers).flat().join(', ')}`;
    case 'plan':
      return a.approve ? 'Plan approved' : a.feedback ? `Kept planning: ${a.feedback}` : 'Kept planning';
  }
}

function RequestRow({ ev, keyboard, onAnswer }: { ev: Ev<'request'>; keyboard: boolean; onAnswer: MessageListProps['onAnswer'] }) {
  if (ev.state === 'open') {
    const plan = ev.requestKind === 'plan' ? planOf(ev.input) : null;
    return (
      <ApprovalCard
        kind={ev.requestKind}
        title={ev.requestKind === 'question' ? 'Claude asks' : ev.requestKind === 'plan' ? 'Approve this plan?' : approvalTitle(ev.tool)}
        subject={ev.requestKind === 'approval' ? ev.title || undefined : undefined}
        description={ev.description}
        canAlways={ev.canAlways}
        questions={ev.requestKind === 'question' ? questionsOf(ev.input) : undefined}
        plan={plan ? <Markdown>{plan}</Markdown> : undefined}
        keyboard={keyboard}
        onAnswer={(a) => onAnswer(ev.id, a)}
      />
    );
  }
  return (
    <div className="flex items-center gap-1.5 px-1 text-xs text-muted-foreground">
      <span className={cn('shrink-0', ev.answer?.type === 'deny' && 'text-destructive')}>{resolvedText(ev)}</span>
      {ev.requestKind === 'approval' && (
        <>
          <span aria-hidden>·</span>
          <span className="shrink-0">{toolVerb(ev.tool)}</span>
          {ev.title && <code className="min-w-0 truncate rounded-sm bg-muted px-1 py-px font-mono text-meta">{ev.title}</code>}
        </>
      )}
    </div>
  );
}

function TurnFooter({ ev }: { ev: Ev<'turn'> }) {
  const took = ev.durationMs != null ? duration(ev.durationMs) : null;
  const text =
    ev.status === 'interrupted'
      ? `Interrupted${took ? ` after ${took}` : ''}`
      : ev.status === 'failed'
        ? `Failed${took ? ` after ${took}` : ''}`
        : took ? `Done in ${took}` : 'Done';
  return (
    <div className="flex items-center gap-2 text-meta text-muted-foreground" data-testid="turn-footer">
      <span className="h-px flex-1 bg-border" aria-hidden />
      <span className={cn('min-w-0 text-center', ev.status === 'failed' && 'text-failed')}>
        {text}
        {ev.reason ? ` · ${ev.reason}` : ''}
      </span>
      <span className="h-px flex-1 bg-border" aria-hidden />
    </div>
  );
}

const timeOf = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });

function NodeView({ node, streams, keyboard = false, onAnswer }: {
  node: RowNode;
  streams: Streams;
  keyboard?: boolean;
  onAnswer?: MessageListProps['onAnswer'];
}) {
  const r = node.row;
  const ev = r.event;
  switch (ev.kind) {
    case 'user_message':
      return (
        <MessageBubble meta={ev.source === 'terminal' ? 'From the terminal' : undefined}>{ev.text}</MessageBubble>
      );
    case 'agent_message': {
      const streamed = r.itemId ? streams[r.itemId]?.markdown : undefined;
      const text = streamed && streamed.length > ev.markdown.length ? streamed : ev.markdown;
      return ev.phase === 'final'
        ? <Markdown>{text}</Markdown>
        : <Markdown className="px-1 text-xs text-muted-foreground">{text}</Markdown>;
    }
    case 'reasoning':
      return ev.summary.trim() ? (
        <details className="px-1 text-xs text-muted-foreground">
          <summary className="cursor-pointer select-none">Thinking</summary>
          <div className="mt-1 border-l border-border pl-2 whitespace-pre-wrap">{ev.summary}</div>
        </details>
      ) : (
        <div className="px-1 text-xs text-muted-foreground">Thought</div>
      );
    case 'tool_call':
      return <ToolCallNode node={node} streams={streams} />;
    case 'request':
      return <RequestRow ev={ev} keyboard={keyboard} onAnswer={onAnswer ?? (async () => {})} />;
    case 'turn':
      return ev.phase === 'ended' ? <TurnFooter ev={ev} /> : null;
    case 'notice':
      return (
        <p className={cn('text-center text-meta text-muted-foreground', ev.level === 'warning' && 'text-waiting')}>
          {ev.text}
          {ev.until ? ` · until ${timeOf(ev.until)}` : ''}
        </p>
      );
    case 'error':
      return <Alert tone="error">{ev.text}</Alert>;
    case 'compaction':
      return (
        <div className="flex items-center gap-2 text-meta text-muted-foreground">
          <span className="h-px flex-1 bg-border" aria-hidden />
          Context compacted
          <span className="h-px flex-1 bg-border" aria-hidden />
        </div>
      );
    case 'plan':
      return (
        <section aria-label="Plan" className="rounded-lg border border-border bg-card p-3">
          <div className="mb-1 text-2xs font-medium tracking-wide text-muted-foreground uppercase">Plan</div>
          <Markdown>{ev.markdown}</Markdown>
        </section>
      );
    case 'unknown':
      return (
        <details className="px-1 text-xs text-muted-foreground">
          <summary className="cursor-pointer select-none">Unrecognised event</summary>
          <ToolOutput className="mt-1">{JSON.stringify(ev.raw, null, 2) ?? ''}</ToolOutput>
        </details>
      );
    case 'usage':
    case 'turn_diff':
      return null;
  }
}

const BlockView = memo(function BlockView({ block, streams, keyboardFor, onAnswer }: {
  block: Block;
  streams: Streams;
  keyboardFor: string | null;
  onAnswer: MessageListProps['onAnswer'];
}) {
  if (block.type === 'group') {
    return (
      <ToolCallRow verb={summarizeTools(block.calls)}>
        {block.nodes.map((n) => (
          <div key={n.row.itemId ?? n.row.seq} className="py-0.5">
            <NodeView node={n} streams={streams} />
          </div>
        ))}
      </ToolCallRow>
    );
  }
  const ev = block.node.row.event;
  return <NodeView node={block.node} streams={streams} keyboard={ev.kind === 'request' && ev.id === keyboardFor} onAnswer={onAnswer} />;
});

/**
 * The conversation: rows in order, pinned to the bottom while new content arrives unless the reader scrolled up
 * ("Jump to latest" then brings them back).
 */
export function MessageList({ rows, provisional, streams, turnRunning, pending, onAnswer, empty, className }: MessageListProps) {
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const blocks = buildBlocks(rows, turnRunning);
  // y / n go to the newest open request only.
  const openRequest = [...rows].reverse().find((r) => r.event.kind === 'request' && r.event.state === 'open');
  const keyboardFor = openRequest?.event.kind === 'request' ? openRequest.event.id : null;

  const toBottom = useCallback(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);
  const onScroll = () => {
    const el = scroller.current;
    if (!el) return;
    const bottom = el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_PX;
    stick.current = bottom;
    setAtBottom(bottom);
  };
  // After every render (new rows, deltas): follow the bottom when pinned.
  useLayoutEffect(() => {
    if (stick.current) toBottom();
  });
  // Content that grows without a render here (images, a row expanding, fonts) also keeps the bottom in view.
  useEffect(() => {
    const el = content.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      if (stick.current) toBottom();
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [toBottom]);
  const jump = () => {
    stick.current = true;
    setAtBottom(true);
    toBottom();
  };

  const nothing = rows.length === 0 && provisional.length === 0 && pending.length === 0 && !turnRunning;
  return (
    <div className={cn('relative flex min-h-0 flex-1 flex-col', className)}>
      <div ref={scroller} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto" role="log" aria-label="Conversation" aria-live="off">
        <div ref={content} className="mx-auto flex w-full max-w-3xl flex-col gap-3 px-4 py-4">
          {nothing && empty}
          {blocks.map((b) => (
            <div key={b.key} className={ROW}>
              <BlockView block={b} streams={streams} keyboardFor={keyboardFor} onAnswer={onAnswer} />
            </div>
          ))}
          {provisional.map((p) => (
            <div key={p.itemId} className={ROW} data-streaming>
              <Markdown>{p.markdown}</Markdown>
            </div>
          ))}
          {pending.map((p) => (
            <MessageBubble key={p.id} tone="pending" label={p.delivery === 'now' ? 'Sending now' : 'Queued'}>
              {p.text}
            </MessageBubble>
          ))}
          {turnRunning && <WorkingRow rows={rows} />}
        </div>
      </div>
      {!atBottom && (
        <Button
          variant="secondary"
          size="sm"
          onClick={jump}
          className="absolute bottom-3 left-1/2 -translate-x-1/2 shadow-popover"
        >
          <ArrowDown aria-hidden className="size-3.5" /> Jump to latest
        </Button>
      )}
    </div>
  );
}
