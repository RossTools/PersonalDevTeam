import { describe, expect, it } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import type { InboxItem } from '../api/gen/InboxItem';
import { lastLine, sessionSnippet } from './TabOverview';

const ev = (seq: number, event: AgentEvent): ConversationEvent => ({ seq, pos: seq, ts: seq, event });
const message = (seq: number, markdown: string) => ev(seq, { kind: 'agent_message', id: `m${seq}`, markdown, phase: 'final' });
const item = (over: Partial<InboxItem>): InboxItem => ({
  id: 'i', kind: 'question', group: 'question', status: 'open', projectId: 'p', ticketId: null, ticketKey: null, ticketTitle: null,
  attemptId: null, sessionId: 's', runId: null, stepId: null, personaId: null, personaName: null, title: 'Which branch?', body: {},
  choices: [], resolution: null, createdAt: 1, resolvedAt: null, ...over,
});

describe('lastLine', () => {
  it('is the last non-empty line, without Markdown marks', () => {
    expect(lastLine('Done.\n\n## Summary\n- Tests **pass** in `web`\n\n')).toBe('Tests pass in web');
    expect(lastLine('> quoted')).toBe('quoted');
    expect(lastLine('  \n ')).toBeNull();
  });
});

describe('sessionSnippet', () => {
  it("is the agent's last line", () => {
    expect(sessionSnippet([message(1, 'First.'), ev(2, { kind: 'reasoning', summary: 'hm' }), message(3, 'Second.\nAll green.')], []))
      .toEqual({ text: 'All green.', question: false });
  });

  it('puts an open question first: the Inbox, then the conversation', () => {
    const request = ev(4, {
      kind: 'request', id: 'r', requestKind: 'approval', tool: 'Bash', title: 'rm -rf target', input: {}, canAlways: true, state: 'open',
    });
    expect(sessionSnippet([message(1, 'Hi'), request], [])).toEqual({ text: 'Wants Bash', question: true });
    expect(sessionSnippet([message(1, 'Hi'), request], [item({})])).toEqual({ text: 'Which branch?', question: true });
    // An answered request is history; a stalled step isn't a question.
    expect(sessionSnippet([message(1, 'Hi'), { ...request, event: { ...request.event, state: 'answered' } as AgentEvent }], [item({ kind: 'stalled' })]))
      .toEqual({ text: 'Hi', question: false });
  });

  it('is null with nothing to say', () => {
    expect(sessionSnippet([], [])).toBeNull();
  });
});
