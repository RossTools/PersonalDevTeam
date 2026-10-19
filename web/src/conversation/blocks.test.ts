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
