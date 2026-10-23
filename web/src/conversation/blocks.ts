import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { rowKey } from './model';

/** A row and the sub-agent calls nested under it (tool calls whose `parentId` is its id). */
export interface RowNode {
  row: ConversationEvent;
  children: RowNode[];
}

/** What the message list shows: single rows, and finished runs of tool calls collapsed under a summary. */
export type Block =
  | { type: 'row'; key: string; node: RowNode }
  | { type: 'group'; key: string; nodes: RowNode[]; calls: { tool: string; title: string }[] };

/** Rows never shown in the list (usage feeds the footer; turn diffs come in 5c; a turn starting says nothing). */
function hidden(e: ConversationEvent): boolean {
  const ev = e.event;
  return ev.kind === 'usage' || ev.kind === 'turn_diff' || (ev.kind === 'turn' && ev.phase === 'started');
}

/** Rows that are part of a turn's activity (they collapse with its tool calls). */
function isActivity(e: ConversationEvent): boolean {
  const ev = e.event;
  return ev.kind === 'tool_call' || ev.kind === 'reasoning' || (ev.kind === 'agent_message' && ev.phase === 'commentary');
}

/**
 * Groups rows (already in `pos` order) into blocks. Tool calls nest under their parent call when it is present.
 * Two or more tool calls in a row (with the reasoning and commentary between them) collapse into one group once
 * their turn has ended, or, for rows without a turn id, once anything follows them or no turn is running.
 */
export function buildBlocks(rows: ConversationEvent[], turnRunning: boolean): Block[] {
  const visible = rows.filter((r) => !hidden(r));
  const callIds = new Set(visible.flatMap((r) => (r.event.kind === 'tool_call' ? [r.event.id] : [])));
  const nodes = new Map<string, RowNode>();
  const top: RowNode[] = [];
  for (const r of visible) {
    const node: RowNode = { row: r, children: [] };
    if (r.event.kind === 'tool_call') nodes.set(r.event.id, node);
  }
  for (const r of visible) {
    const node = r.event.kind === 'tool_call' ? nodes.get(r.event.id)! : { row: r, children: [] };
    const parent = r.event.kind === 'tool_call' && r.event.parentId && r.event.parentId !== r.event.id && callIds.has(r.event.parentId)
      ? nodes.get(r.event.parentId)
      : undefined;
    if (parent) parent.children.push(node);
    else top.push(node);
  }

  const ended = new Set(
    rows.flatMap((r) => (r.event.kind === 'turn' && r.event.phase === 'ended' && r.turnId ? [r.turnId] : [])),
  );
  const blocks: Block[] = [];
  let run: RowNode[] = [];
  const flush = (atEnd: boolean) => {
    if (!run.length) return;
    const calls = run.flatMap((n) => (n.row.event.kind === 'tool_call' ? [{ tool: n.row.event.tool, title: n.row.event.title }] : []));
    const turnId = run[0].row.turnId;
    const done = turnId ? ended.has(turnId) : !turnRunning || !atEnd;
    const running = run.some((n) => n.row.event.kind === 'tool_call' && n.row.event.status === 'running');
    if (calls.length >= 2 && done && !running) {
      blocks.push({ type: 'group', key: `g:${rowKey(run[0].row)}`, nodes: run, calls });
    } else {
      for (const n of run) blocks.push({ type: 'row', key: rowKey(n.row), node: n });
    }
    run = [];
  };
  for (const n of top) {
    if (isActivity(n.row)) {
      // A new turn starts a new run.
      if (run.length && run[0].row.turnId !== n.row.turnId) flush(false);
      run.push(n);
      continue;
    }
    flush(false);
    blocks.push({ type: 'row', key: rowKey(n.row), node: n });
  }
  flush(true);
  return blocks;
}

function sameNode(a: RowNode, b: RowNode): boolean {
  return a.row === b.row && a.children.length === b.children.length && a.children.every((c, i) => sameNode(c, b.children[i]));
}

function sameBlock(a: Block, b: Block): boolean {
  if (a.type === 'row' && b.type === 'row') return a.key === b.key && sameNode(a.node, b.node);
  if (a.type === 'group' && b.type === 'group') {
    return a.key === b.key && a.nodes.length === b.nodes.length && a.nodes.every((n, i) => sameNode(n, b.nodes[i]));
  }
  return false;
}

/**
 * `next`, with each block that shows the same rows as one in `prev` replaced by that one: a new or updated row makes
 * new objects for its own block only, so the others' views can skip rendering.
 */
export function reuseBlocks(prev: Block[], next: Block[]): Block[] {
  const old = new Map(prev.map((b) => [b.key, b]));
  return next.map((b) => {
    const o = old.get(b.key);
    return o && sameBlock(o, b) ? o : b;
  });
}

/** The item ids a block shows (their streamed text is the block's). */
export function blockItemIds(b: Block): string[] {
  const out: string[] = [];
  const walk = (n: RowNode) => {
    if (n.row.itemId) out.push(n.row.itemId);
    n.children.forEach(walk);
  };
  if (b.type === 'row') walk(b.node);
  else b.nodes.forEach(walk);
  return out;
}

/** The tool call a working turn is busy with: the last running top-level call, if any. */
export function currentTool(rows: ConversationEvent[]): ConversationEvent | null {
  for (let i = rows.length - 1; i >= 0; i--) {
    const ev = rows[i].event;
    if (ev.kind === 'tool_call' && ev.status === 'running' && !ev.parentId) return rows[i];
  }
  return null;
}

/** When the running turn started (the last `turn started` row's time), if known. */
export function turnStartedAt(rows: ConversationEvent[]): number | null {
  for (let i = rows.length - 1; i >= 0; i--) {
    const ev = rows[i].event;
    if (ev.kind === 'turn') return ev.phase === 'started' ? rows[i].ts : null;
  }
  return null;
}

/** The latest usage report, for the footer's context readout. */
export function latestUsage(rows: ConversationEvent[]) {
  let best: ConversationEvent | null = null;
  for (const r of rows) if (r.event.kind === 'usage' && (!best || r.seq > best.seq)) best = r;
  return best?.event.kind === 'usage' ? best.event : null;
}
