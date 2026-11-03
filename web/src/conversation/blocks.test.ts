import { describe, expect, it } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { buildBlocks, currentTool } from './blocks';
import { lookedUp } from './pdtTools';
import { approvalTitle, clock, questionsOf, summarizeTools, toolVerb } from './tools';

const ev = (seq: number, event: AgentEvent, turnId = 'u1'): ConversationEvent => ({ seq, pos: seq, ts: seq, event, turnId, itemId: `i${seq}` });
const call = (id: string, tool: string, title: string, status: 'running' | 'done' = 'done', parentId?: string): AgentEvent =>
  ({ kind: 'tool_call', id, tool, title, input: {}, status, ...(parentId ? { parentId } : {}) });

describe('tool labels', () => {
  it('names each tool by what it does', () => {
    expect(toolVerb('Bash')).toBe('Run');
    expect(toolVerb('Edit')).toBe('Edit');
    expect(toolVerb('Glob')).toBe('Search');
    expect(toolVerb('Task')).toBe('Agent:');
    expect(toolVerb('mcp__github__create_issue')).toBe('github: create_issue');
    expect(toolVerb('Mystery')).toBe('Mystery');
    expect(approvalTitle('Bash')).toBe('Run this command?');
    expect(approvalTitle('Edit')).toBe('Edit this file?');
    expect(approvalTitle('Write')).toBe('Write this file?');
  });

  it('summarises a run of calls, counting files once', () => {
    expect(summarizeTools([
      { tool: 'Bash', title: 'ls' }, { tool: 'Bash', title: 'cargo test' }, { tool: 'Bash', title: 'git status' },
      { tool: 'Edit', title: 'a.ts' }, { tool: 'Edit', title: 'a.ts' }, { tool: 'Write', title: 'b.ts' },
    ])).toBe('Ran 3 commands, edited 2 files');
    expect(summarizeTools([{ tool: 'Read', title: 'x' }, { tool: 'Grep', title: 'foo' }])).toBe('Read 1 file, searched 1 time');
  });

  it('names a run of one uncategorised tool by the tool and its calls', () => {
    const search = { tool: 'mcp__pdt__search_tickets', title: '' };
    expect(summarizeTools([search, search])).toBe('search_tickets · 2 calls');
    expect(summarizeTools([search, { tool: 'mcp__pdt__get_ticket', title: '' }])).toBe('Used 2 tools');
  });

  it('formats a running clock', () => {
    expect(clock(42_000)).toBe('0:42');
    expect(clock(3_723_000)).toBe('1:02:03');
  });

  it('reads AskUserQuestion input', () => {
    expect(questionsOf({ questions: [{ question: 'Which?', header: 'Lib', options: [{ label: 'A', description: 'a' }, { label: 'B' }], multiSelect: true }] }))
      .toEqual([{ question: 'Which?', header: 'Lib', options: [{ label: 'A', description: 'a' }, { label: 'B' }], multiSelect: true }]);
    expect(questionsOf('nonsense')).toEqual([]);
  });
});

describe('buildBlocks', () => {
  const turn = (seq: number, phase: 'started' | 'ended'): ConversationEvent =>
    ev(seq, { kind: 'turn', phase, ...(phase === 'ended' ? { status: 'completed' as const, durationMs: 1000 } : {}) });

  it('collapses two or more tool calls once their turn ended', () => {
    const rows = [
      turn(1, 'started'),
      ev(2, call('a', 'Bash', 'ls')),
      ev(3, { kind: 'agent_message', id: 'c', markdown: 'checking', phase: 'commentary' }),
      ev(4, call('b', 'Edit', 'x.ts')),
      ev(5, { kind: 'agent_message', id: 'f', markdown: 'done', phase: 'final' }),
      turn(6, 'ended'),
    ];
    const blocks = buildBlocks(rows, false);
    expect(blocks.map((b) => b.type)).toEqual(['group', 'row', 'row']);
    expect(blocks[0].type === 'group' && blocks[0].nodes).toHaveLength(3);
  });

  // Tools, the answer, then more tools and a closing line: the answer was commentary (a tool call followed it).
  const answerTurn = (): ConversationEvent[] => [
    turn(1, 'started'),
    ev(2, call('a', 'Bash', 'gh cache list')),
    ev(3, { kind: 'agent_message', id: 'n', markdown: 'Checking pricing…', phase: 'commentary' }),
    ev(4, call('b', 'WebSearch', 'pricing')),
    ev(5, call('c', 'WebSearch', 'r2 pricing')),
    ev(6, { kind: 'agent_message', id: 'm', markdown: 'We are at the limit.\n\n| Option | Price |\n| --- | --- |\n| A | $1 |', phase: 'commentary' }),
    ev(7, call('d', 'mcp__pdt__list_watches', '')),
    ev(8, call('e', 'mcp__pdt__get_ticket', '')),
    ev(9, { kind: 'agent_message', id: 'f', markdown: 'Added to the ticket.', phase: 'final' }),
    turn(10, 'ended'),
  ];
  const shape = (blocks: ReturnType<typeof buildBlocks>) =>
    blocks.map((b) => (b.type === 'row' ? `row(${b.node.row.seq})` : `${b.type}(${b.nodes.map((n) => n.row.seq).join(',')})`));

  it('never folds the answer into a tool group, only a one-line narration before a call', () => {
    const blocks = buildBlocks(answerTurn(), false);
    expect(shape(blocks)).toEqual(['group(2,3,4,5)', 'row(6)', 'group(7,8)', 'row(9)', 'row(10)']);
    const summaries = blocks.flatMap((b) => (b.type === 'group' ? [summarizeTools(b.calls)] : []));
    expect(summaries).toEqual(['Ran 1 command, looked up 2 pages', 'Used 2 tools']);
  });

  it("shows PDT's actions as cards of their own, consecutive tickets made in one", () => {
    const rows = [
      turn(1, 'started'),
      ev(2, call('a', 'Read', 'src/a.ts')),
      ev(3, call('b', 'mcp__pdt__create_ticket', '')),
      ev(4, call('c', 'mcp:pdt/create_ticket', '')),
      ev(5, call('d', 'Grep', 'cart')),
      ev(6, call('e', 'Read', 'src/b.ts')),
      ev(7, call('f', 'mcp__pdt__link_tickets', '')),
      ev(8, call('g', 'mcp__pdt__send_to_session', '')),
      ev(9, call('h', 'mcp__pdt__create_ticket', '')),
      turn(10, 'ended'),
      ev(11, call('i', 'mcp__pdt__create_ticket', ''), 'u2'),
    ];
    expect(shape(buildBlocks(rows, false))).toEqual([
      'row(2)', 'tickets(3,4)', 'group(5,6)', 'row(7)', 'row(8)', 'row(9)', 'row(10)', 'row(11)',
    ]);
  });

  it('never folds stopping work, issues, documents or notes in with the turn\'s other tools, by either name', () => {
    const tools = [
      'mcp__pdt__stop_ticket', 'mcp:pdt/issue_create', 'mcp__pdt__report_bug', 'mcp__pdt__link_issue', 'mcp:pdt/unlink_issue',
      'mcp__pdt__document_create', 'mcp:pdt/doc_edit', 'mcp__pdt__sheet_edit', 'mcp__pdt__document_link', 'mcp__pdt__save_note',
      'mcp:pdt/delete_note',
    ];
    const rows = [
      turn(1, 'started'),
      ...tools.flatMap((t, i) => [ev(2 + 2 * i, call(`r${i}`, 'Read', `f${i}.ts`)), ev(3 + 2 * i, call(`p${i}`, t, ''))]),
      turn(100, 'ended'),
    ];
    expect(buildBlocks(rows, false).filter((b) => b.type === 'group')).toEqual([]);
  });

  it('names the tickets folded lookups read, each once', () => {
    const out = (v: unknown) => JSON.stringify(v);
    expect(lookedUp([
      { tool: 'mcp__pdt__get_ticket', input: { ticket: 'pdt-499' }, output: out({ key: 'PDT-499', title: 'x' }) },
      { tool: 'mcp:pdt/get_ticket', input: {}, output: out({ structuredContent: { key: 'PDT-492' } }) },
      { tool: 'mcp__pdt__search_tickets', input: { query: 'cart' }, output: out({ tickets: [{ key: 'PDT-492' }, { key: 'PDT-12' }], total: 2 }) },
      { tool: 'mcp__pdt__get_overview', input: {}, output: out({ projects: [], inProgress: [{ key: 'PDT-7' }], inbox: [] }) },
    ])).toEqual(['PDT-499', 'PDT-492', 'PDT-12', 'PDT-7']);
    // A lookup that hasn't answered names the ticket it asked for.
    expect(lookedUp([{ tool: 'mcp__pdt__get_ticket', input: { ticket: 'PDT-3' } }])).toEqual(['PDT-3']);
    // Anything else in the run, or no tickets read: the row keeps its summary.
    expect(lookedUp([{ tool: 'mcp__pdt__get_ticket', input: { ticket: 'PDT-3' } }, { tool: 'Bash', input: {} }])).toBeNull();
    expect(lookedUp([{ tool: 'mcp__pdt__search_tickets', input: {}, output: out({ tickets: [], total: 0 }) }])).toBeNull();
    expect(lookedUp([])).toBeNull();
  });

  it('keeps commentary at top level when no tool call follows it directly', () => {
    const rows = [
      turn(1, 'started'),
      ev(2, call('a', 'Bash', 'ls')),
      ev(3, call('b', 'Bash', 'pwd')),
      ev(4, { kind: 'agent_message', id: 'c', markdown: 'short', phase: 'commentary' }),
      ev(5, { kind: 'reasoning', summary: 'thinking' }),
      ev(6, call('c', 'Bash', 'git status')),
      turn(7, 'ended'),
    ];
    expect(shape(buildBlocks(rows, false))).toEqual(['group(2,3)', 'row(4)', 'row(5)', 'row(6)', 'row(7)']);
  });

  it('shows text at top level while the turn streams, and the same structure once it ends and on reload', () => {
    const all = answerTurn();
    for (let n = 2; n < all.length; n++) {
      const live = buildBlocks(all.slice(0, n), true);
      for (const b of live) {
        if (b.type === 'group') expect(b.nodes.every((x) => x.row.event.kind !== 'agent_message' || x.row.seq === 3)).toBe(true);
      }
      // Every row seen so far is shown, in order.
      expect(live.flatMap((b) => (b.type === 'row' ? [b.node.row.seq] : b.nodes.map((x) => x.row.seq))))
        .toEqual(all.slice(1, n).map((r) => r.seq));
    }
    expect(shape(buildBlocks(all, true))).toEqual(shape(buildBlocks(all, false)));
  });

  it('keeps calls separate while their turn runs', () => {
    const rows = [turn(1, 'started'), ev(2, call('a', 'Bash', 'ls')), ev(3, call('b', 'Bash', 'pwd', 'running'))];
    expect(buildBlocks(rows, true).map((b) => b.type)).toEqual(['row', 'row']);
    expect(currentTool(rows)?.seq).toBe(3);
  });

  it('nests sub-agent calls under their parent and hides usage and turn starts', () => {
    const rows = [
      turn(1, 'started'),
      ev(2, call('task', 'Task', 'explore')),
      ev(3, call('inner', 'Read', 'a.ts', 'done', 'task')),
      ev(4, { kind: 'usage', rateLimits: [] }),
    ];
    const blocks = buildBlocks(rows, true);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].type === 'row' && blocks[0].node.children.map((c) => c.row.seq)).toEqual([3]);
  });
});
