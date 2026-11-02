import { createContext, memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ArrowDown } from 'lucide-react';
import {
  Alert,
  ApprovalCard,
  Button,
  Dialog,
  DiffStat,
  ImageThumb,
  DiffViewer,
  diffStats,
  MessageBubble,
  parseDiff,
  Spinner,
  ToolCallRow,
  ToolOutput,
  relativeTime,
  type ApprovalAnswer,
} from '@ui';
import { cn } from '@ui/lib/utils';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { AgentKind } from '../api/gen/AgentKind';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { MessageSource } from '../api/gen/MessageSource';
import type { PdtMessage } from '../api/gen/PdtMessage';
import type { PendingMessage } from '../api/gen/PendingMessage';
import { Markdown } from '../common/Markdown';
import { MarkdownRefsContext, type MarkdownRefs } from '../common/markdownRefs';
import { blockItemIds, buildBlocks, currentTool, reuseBlocks, turnStartedAt, type Block, type RowNode } from './blocks';
import type { Streams } from './model';
import { CreateTicketsCard, PdtToolCard, type PdtCall } from './PdtToolCard';
import { PdtProposal } from './PdtProposal';
import { isCardTool, pdtApprovalTitle } from './pdtTools';
import { approvalTitle, clock, duration, inputText, planOf, questionsOf, summarizeTools, toolVerb } from './tools';

type Ev<K extends AgentEvent['kind']> = Extract<AgentEvent, { kind: K }>;

/** Said under a user message that the user didn't send from the composer. */
const SOURCE_META: Partial<Record<MessageSource, string>> = {
  terminal: 'From the terminal',
  notice: 'Sent automatically: news about the pull request',
};

/** The one line PDT's own messages to a PM conversation collapse to (PDT-211). */
const PDT_MESSAGE: Record<PdtMessage, string> = {
  update: 'PDT brought this conversation up to date',
  checkpoint: 'PDT asked for a journal checkpoint',
  journal: "PDT asked for this conversation's journal entry",
};

/** One of PDT's own messages to a PM conversation: a muted line that opens to the full text the PM got. */
export function PdtMessageRow({ kind, text, when }: { kind: PdtMessage; text: string; when: string }) {
  return (
    <details className="px-1 text-meta text-muted-foreground">
      <summary className="cursor-pointer select-none">
        {PDT_MESSAGE[kind]} · {when}
      </summary>
      <div className="mt-1 border-l border-border pl-2 text-sm text-foreground">
        <Markdown>{text}</Markdown>
      </div>
    </details>
  );
}

export interface MessageListProps {
  rows: ConversationEvent[];
  provisional: { itemId: string; markdown: string }[];
  streams: Streams;
  turnRunning: boolean;
  pending: PendingMessage[];
  /** Answers an open request (`request` event `id`). */
  onAnswer: (requestId: string, answer: ApprovalAnswer) => Promise<unknown>;
  /** Where a message's attached image is served (by attachment id). */
  imageUrl?: (attachmentId: string) => string;
  /** Which agent: Claude's plans can be approved straight into Accept edits. */
  agent?: AgentKind;
  /** Offered under the last turn when it was interrupted (a restart, a crash, Stop): picks up where it left off. */
  onContinue?: () => Promise<unknown>;
  /** Shown when there is nothing yet. */
  empty?: ReactNode;
  /** Where the conversation is: its ticket keys, PRs, file paths and SHAs show as pills and links (PDT-382). */
  refs?: MarkdownRefs | null;
  className?: string;
}

/** What rows need from the whole conversation, without threading it through every block. */
interface ListContext {
  /** Each turn's diff, by turn id. */
  turnDiffs: Map<string, string>;
  /** The item id of the last ended turn, when it may be continued. */
  continueTurn: string | null;
  onContinue?: () => Promise<unknown>;
  agent: AgentKind;
  imageUrl?: (attachmentId: string) => string;
}
const ListCtx = createContext<ListContext>({ turnDiffs: new Map(), continueTurn: null, agent: 'claude' });

/** PDT's attachment ids are UUIDs; anything else is the agent's own note ("image/png") with nothing to show. */
const isAttachment = (s: string) => /^[0-9a-f-]{36}$/i.test(s);

function Images({ images }: { images: string[] }) {
  const { imageUrl } = useContext(ListCtx);
  if (!images.length) return null;
  return (
    <>
      {images.map((img, i) => {
        const src = isAttachment(img) && imageUrl ? imageUrl(img) : undefined;
        return <ImageThumb key={`${img}-${i}`} src={src} href={src} alt={`Image ${i + 1}`} />;
      })}
    </>
  );
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

/** A PDT tool call as its card wants it, with its streamed output. */
function pdtCall(node: RowNode, streams: Streams): PdtCall {
  const ev = node.row.event as Ev<'tool_call'>;
  return { tool: ev.tool, input: ev.input, status: ev.status, output: (node.row.itemId && streams[node.row.itemId]?.output) || ev.output };
}

function ToolCallNode({ node, streams }: { node: RowNode; streams: Streams }) {
  const ev = node.row.event as Ev<'tool_call'>;
  // PDT's own actions show as cards (PDT-382); in a sub-agent, as rows like the rest.
  if (isCardTool(ev.tool) && !ev.parentId) return <PdtToolCard call={pdtCall(node, streams)} />;
  const output = (node.row.itemId && streams[node.row.itemId]?.output) || ev.output;
  // A file change shows its diff instead of the raw input, and its output only when it failed.
  const diff = ev.diff || null;
  const input = diff ? '' : inputText(ev.tool, ev.input);
  const shownOutput = diff && ev.status !== 'failed' ? '' : output;
  const stats = diff ? diffStats(diff) : null;
  const meta =
    ev.exitCode != null && ev.exitCode !== 0
      ? `exit ${ev.exitCode}`
      : stats && ev.status === 'done'
        ? <DiffStat added={stats.added} removed={stats.removed} />
        : undefined;
  return (
    <ToolCallRow
      verb={toolVerb(ev.tool)}
      subject={ev.title || undefined}
      status={ev.status}
      meta={meta}
      details={
        input || shownOutput || diff ? (
          <>
            {input && <ToolOutput label="Input">{input}</ToolOutput>}
            {diff && <DiffViewer diff={diff} hideHeaders={!!ev.title && parseDiff(diff).length === 1} className="max-h-96 overflow-auto" />}
            {shownOutput && <ToolOutput label="Output" tone={ev.status === 'failed' ? 'error' : 'default'}>{shownOutput}</ToolOutput>}
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
  // Codex: answered in its TUI, which PDT only hears about as resolved.
  if (!a) return 'Answered in the terminal';
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

/** The DOM id of an open request's card, for the pinned approval bar's Review. */
export const requestAnchor = (requestId: string) => `request-${requestId}`;

function RequestRow({ ev, keyboard, onAnswer }: { ev: Ev<'request'>; keyboard: boolean; onAnswer: MessageListProps['onAnswer'] }) {
  const { agent } = useContext(ListCtx);
  if (ev.state === 'open') {
    const plan = ev.requestKind === 'plan' ? planOf(ev.input) : null;
    const args = ev.requestKind === 'approval' && !ev.diff ? inputText(ev.tool, ev.input) : '';
    // PDT's own tools say what they will create or change (PDT-382).
    const pdtTitle = ev.requestKind === 'approval' ? pdtApprovalTitle(ev.tool, ev.input) : null;
    return (
      <ApprovalCard
        id={requestAnchor(ev.id)}
        kind={ev.requestKind}
        title={ev.requestKind === 'question' ? 'A question for you' : ev.requestKind === 'plan' ? 'Approve this plan?' : pdtTitle ?? approvalTitle(ev.tool)}
        subject={ev.requestKind === 'approval' && !ev.diff && !pdtTitle ? ev.title || undefined : undefined}
        preview={ev.diff ? <DiffViewer diff={ev.diff} /> : pdtTitle ? <PdtProposal tool={ev.tool} input={ev.input} /> : undefined}
        planAcceptEdits={agent === 'claude'}
        description={ev.description}
        details={args && args !== '{}' && args !== ev.title ? <ToolOutput>{args}</ToolOutput> : undefined}
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

function ChangesButton({ diff }: { diff: string }) {
  const [open, setOpen] = useState(false);
  const stats = diffStats(diff);
  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)} className="h-auto px-1.5 py-0.5 text-meta">
        Changes <DiffStat added={stats.added} removed={stats.removed} />
      </Button>
      <Dialog open={open} onOpenChange={setOpen} title="Changes in this turn" size="xl">
        <DiffViewer diff={diff} />
      </Dialog>
    </>
  );
}

function ContinueButton({ onContinue }: { onContinue: () => Promise<unknown> }) {
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    try {
      await onContinue();
    } finally {
      setBusy(false);
    }
  };
  return (
    <Button variant="secondary" size="sm" pending={busy} disabled={busy} onClick={() => void go()}>
      Continue
    </Button>
  );
}

function TurnFooter({ ev, row }: { ev: Ev<'turn'>; row: ConversationEvent }) {
  const { turnDiffs, continueTurn, onContinue } = useContext(ListCtx);
  const diff = row.turnId ? turnDiffs.get(row.turnId) : undefined;
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
        {ev.reason && ev.reason.toLowerCase() !== ev.status ? ` · ${ev.reason}` : ''}
      </span>
      {diff && <ChangesButton diff={diff} />}
      {onContinue && continueTurn !== null && continueTurn === row.itemId && <ContinueButton onContinue={onContinue} />}
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
      if (ev.pdt) return <PdtMessageRow kind={ev.pdt} text={ev.text} when={relativeTime(r.ts)} />;
      return (
        <MessageBubble meta={SOURCE_META[ev.source]} images={ev.images.length ? <Images images={ev.images} /> : undefined}>
          {ev.text || undefined}
        </MessageBubble>
      );
    case 'agent_message': {
      const streamed = r.itemId ? streams[r.itemId]?.markdown : undefined;
      const text = streamed && streamed.length > ev.markdown.length ? streamed : ev.markdown;
      // Commentary (the short replies between tool calls) reads like the final answer: it is often what the agent
      // most wants seen.
      return <Markdown>{text}</Markdown>;
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
      return ev.phase === 'ended' ? <TurnFooter ev={ev} row={r} /> : null;
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
      return <Divider>Context compacted</Divider>;
    case 'conversation_reset':
      return <Divider>{resetLabel(ev.trigger)}</Divider>;
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

function Divider({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 text-meta text-muted-foreground">
      <span className="h-px flex-1 bg-border" aria-hidden />
      {children}
      <span className="h-px flex-1 bg-border" aria-hidden />
    </div>
  );
}

/** What a `conversation_reset` divider says: Claude's `/clear`, the TUI's `/resume`, or whatever else started over. */
function resetLabel(trigger: string): string {
  switch (trigger) {
    case 'clear':
      return 'Conversation cleared';
    case 'resume':
      return 'Switched to another conversation';
    default:
      return `New conversation (${trigger})`;
  }
}

/**
 * One block. It renders again only when its own rows, its own streamed text or the keyboard's request changes: a
 * streamed token re-renders its block, not the conversation (PDT-136).
 */
const BlockView = memo(
  function BlockView({ block, streams, keyboardFor, onAnswer }: {
    block: Block;
    streams: Streams;
    keyboardFor: string | null;
    onAnswer: MessageListProps['onAnswer'];
  }) {
    if (block.type === 'tickets') return <CreateTicketsCard calls={block.nodes.map((n) => pdtCall(n, streams))} />;
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
  },
  (a, b) =>
    a.block === b.block && a.keyboardFor === b.keyboardFor && a.onAnswer === b.onAnswer
    && (a.streams === b.streams || blockItemIds(b.block).every((id) => a.streams[id] === b.streams[id])),
);

/** The rows' blocks, keeping each unchanged block the same object from render to render. */
function useBlocks(rows: ConversationEvent[], turnRunning: boolean): Block[] {
  const prev = useRef<Block[]>([]);
  return useMemo(() => {
    const next = reuseBlocks(prev.current, buildBlocks(rows, turnRunning));
    prev.current = next;
    return next;
  }, [rows, turnRunning]);
}

/** The latest `f`, behind a function that never changes (callers pass new closures every render). */
function useLatest<A extends unknown[], R>(f: (...args: A) => R): (...args: A) => R;
function useLatest<A extends unknown[], R>(f: ((...args: A) => R) | undefined): ((...args: A) => R) | undefined;
function useLatest<A extends unknown[], R>(f: ((...args: A) => R) | undefined) {
  const ref = useRef(f);
  useLayoutEffect(() => {
    ref.current = f;
  });
  const stable = useCallback((...args: A) => ref.current!(...args), []);
  return f ? stable : undefined;
}

/** The last row matching `f`, without copying the rows. */
function findLast(rows: ConversationEvent[], f: (r: ConversationEvent) => boolean): ConversationEvent | undefined {
  for (let i = rows.length - 1; i >= 0; i--) if (f(rows[i])) return rows[i];
  return undefined;
}

/** `next`, or `prev` when it holds the same diffs (so the context, and every row reading it, stays put). */
function sameDiffs(prev: Map<string, string>, next: Map<string, string>): Map<string, string> {
  if (prev.size !== next.size) return next;
  for (const [k, v] of next) if (prev.get(k) !== v) return next;
  return prev;
}

/**
 * The conversation: rows in order, pinned to the bottom while new content arrives unless the reader scrolled up
 * ("Jump to latest" then brings them back).
 */
export function MessageList({
  rows,
  provisional,
  streams,
  turnRunning,
  pending,
  onAnswer,
  agent = 'claude',
  onContinue,
  imageUrl,
  empty,
  refs = null,
  className,
}: MessageListProps) {
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const blocks = useBlocks(rows, turnRunning);
  const answer = useLatest(onAnswer);
  const continueLatest = useLatest(onContinue);
  // y / n go to the newest open request only.
  const keyboardFor = useMemo(() => {
    const open = findLast(rows, (r) => r.event.kind === 'request' && r.event.state === 'open');
    return open?.event.kind === 'request' ? open.event.id : null;
  }, [rows]);
  const diffs = useRef(new Map<string, string>());
  const turnDiffs = useMemo(() => {
    const m = new Map<string, string>();
    for (const r of rows) if (r.event.kind === 'turn_diff' && r.turnId && r.event.diff) m.set(r.turnId, r.event.diff);
    diffs.current = sameDiffs(diffs.current, m);
    return diffs.current;
  }, [rows]);
  // Continue: the conversation's last turn was cut off and nothing has happened since.
  const continueTurn = useMemo(() => {
    const last = findLast(rows, (r) => r.event.kind === 'turn' || r.event.kind === 'user_message');
    const cut = last?.event.kind === 'turn' && last.event.phase === 'ended' && last.event.status === 'interrupted';
    return cut && !turnRunning && pending.length === 0 ? last.itemId ?? null : null;
  }, [rows, turnRunning, pending.length]);
  const ctx = useMemo<ListContext>(
    () => ({ turnDiffs, continueTurn, onContinue: continueLatest, agent, imageUrl }),
    [turnDiffs, continueTurn, continueLatest, agent, imageUrl],
  );

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
    <MarkdownRefsContext.Provider value={refs}>
    <ListCtx.Provider value={ctx}>
    <div className={cn('relative flex min-h-0 flex-1 flex-col', className)}>
      <div ref={scroller} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto" role="log" aria-label="Conversation" aria-live="off">
        <div ref={content} className="mx-auto flex w-full max-w-3xl flex-col gap-3 px-4 py-4">
          {nothing && empty}
          {blocks.map((b) => (
            <div key={b.key} className={ROW}>
              <BlockView block={b} streams={streams} keyboardFor={keyboardFor} onAnswer={answer} />
            </div>
          ))}
          {provisional.map((p) => (
            <div key={p.itemId} className={ROW} data-streaming>
              <Markdown>{p.markdown}</Markdown>
            </div>
          ))}
          {pending.map((p) => p.pdt ? (
            <PdtMessageRow key={p.id} kind={p.pdt} text={p.text} when="queued" />
          ) : (
            <MessageBubble
              key={p.id}
              tone="pending"
              label={p.delivery === 'now' ? 'Sending now' : 'Queued'}
              images={p.images.length ? <Images images={p.images} /> : undefined}
            >
              {p.text || undefined}
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
    </ListCtx.Provider>
    </MarkdownRefsContext.Provider>
  );
}
