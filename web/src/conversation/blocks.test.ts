import { describe, expect, it } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { buildBlocks, currentTool } from './blocks';
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
    ev(7, call('d', 'mcp__pdt__add_ticket_comment', '')),
    ev(8, call('e', 'mcp__pdt__get_ticket', '')),
    ev(9, { kind: 'agent_message', id: 'f', markdown: 'Added to the ticket.', phase: 'final' }),
    turn(10, 'ended'),
  ];
  const shape = (blocks: ReturnType<typeof buildBlocks>) =>
    blocks.map((b) => (b.type === 'group' ? `group(${b.nodes.map((n) => n.row.seq).join(',')})` : `row(${b.node.row.seq})`));

  it('never folds the answer into a tool group, only a one-line narration before a call', () => {
    const blocks = buildBlocks(answerTurn(), false);
    expect(shape(blocks)).toEqual(['group(2,3,4,5)', 'row(6)', 'group(7,8)', 'row(9)', 'row(10)']);
    const summaries = blocks.flatMap((b) => (b.type === 'group' ? [summarizeTools(b.calls)] : []));
    expect(summaries).toEqual(['Ran 1 command, looked up 2 pages', 'Used 2 tools']);
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
      expect(live.flatMap((b) => (b.type === 'group' ? b.nodes.map((x) => x.row.seq) : [b.node.row.seq])))
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
