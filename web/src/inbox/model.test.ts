// The Inbox's parsers of an item's untyped `body`: what each reads, and what it falls back to when a field is missing
// or has the wrong type (a server change must not crash the Inbox).
import { describe, expect, it } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import {
  changeRequestLabel, confirmOf, decideRowsOf, gateQuestion, itemLabel, longTitle, noticeText, outcomeTone, passesOf,
  proposalOf, proposalPath, resends, selectedUpdate, ticketProposalOf, triageOf, updateSelection,
} from './model';

const item = (over: Partial<InboxItem> = {}): InboxItem => ({
  id: 'i', kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, changeRequest: null, title: 'Title',
  body: {}, choices: [], resolution: null, createdAt: 0, resolvedAt: null, ...over,
});
const body = (b: Record<string, unknown>, over: Partial<InboxItem> = {}) => item({ body: b, ...over });

describe('triageOf', () => {
  it.each([
    ['all fields', { summary: 'Notes', issue: 'GH-4', external: true }, { summary: 'Notes', issue: 'GH-4', external: true }],
    ['missing fields', {}, { summary: null, issue: null, external: false }],
    ['wrong types', { summary: 3, issue: ['GH-4'], external: 'true' }, { summary: null, issue: null, external: false }],
  ])('%s', (_, b, want) => expect(triageOf(body(b))).toEqual(want));
});

describe('confirmOf', () => {
  it('reads the tool, agent, session, input and secret note', () => {
    expect(confirmOf(body({ tool: 'create_ticket', agent: 'Codex', sessionTitle: 'PM', input: { a: 1 }, secretNote: 'Holds TOKEN' }))).toEqual({
      tool: 'create_ticket', agent: 'Codex', session: 'PM', input: '{\n  "a": 1\n}', secretNote: 'Holds TOKEN',
    });
  });

  it.each([
    ['missing fields', {}],
    ['wrong types', { tool: 1, agent: {}, sessionTitle: 5, secretNote: false }],
    ['a blank session title', { sessionTitle: '' }],
  ])('%s: the title for the tool, an empty input and nulls', (_, b) => {
    expect(confirmOf(body(b, { title: 'Agent call' }))).toEqual({ tool: 'Agent call', agent: null, session: null, input: '{}', secretNote: null });
  });
});

describe('decideRowsOf', () => {
  it.each([
    ['rows', { rows: [{ itemId: 'i1' }] }, [{ itemId: 'i1' }]],
    ['no rows: a plain call', {}, null],
    ['rows that are not a list', { rows: { itemId: 'i1' } }, null],
    ['an empty list', { rows: [] }, []],
  ])('%s', (_, b, want) => expect(decideRowsOf(body(b))).toEqual(want));
});

describe('proposalOf and proposalPath', () => {
  it('reads a workflow proposal', () => {
    const p = body({ workflowId: 'w1', workflow: 'Ship', version: 3, note: 'Faster', summary: '+1 step' });
    expect(proposalOf(p)).toEqual({ workflowId: 'w1', workflow: 'Ship', version: 3, note: 'Faster', summary: '+1 step' });
    expect(proposalPath(p)).toBe('/workflows/w1/versions');
  });

  it.each([
    ['missing fields', {}],
    ['wrong types', { workflowId: 1, workflow: null, version: '3', note: [], summary: {} }],
  ])('%s: empty strings and version 0', (_, b) => {
    expect(proposalOf(body(b))).toEqual({ workflowId: '', workflow: '', version: 0, note: '', summary: '' });
  });
});

describe('ticketProposalOf', () => {
  it('reads a ticket proposal, keeping only string labels', () => {
    const p = body({
      description: 'Do it', priority: 'high', labels: ['ui', 3, null, 'web'], reason: 'Found it', persona: 'Coder',
      fromTicket: 'PDT-1', fromTicketId: 't1',
    });
    expect(ticketProposalOf(p)).toEqual({
      description: 'Do it', priority: 'high', labels: ['ui', 'web'], reason: 'Found it', persona: 'Coder', fromTicket: 'PDT-1', fromTicketId: 't1',
    });
  });

  it.each([
    ['missing fields', {}],
    ['wrong types', { description: 1, priority: 2, labels: 'ui', reason: 3, persona: {}, fromTicket: [], fromTicketId: true }],
    ['blank strings', { description: '', reason: '  ', persona: '', fromTicket: ' ', fromTicketId: '' }],
  ])('%s: no description, priority none, no labels and nulls', (_, b) => {
    expect(ticketProposalOf(body(b))).toEqual({
      description: '', priority: 'none', labels: [], reason: null, persona: null, fromTicket: null, fromTicketId: null,
    });
  });
});

describe('updateSelection and selectedUpdate', () => {
  it('round-trips an Update’s id and tells it from a Needs-you item', () => {
    expect(selectedUpdate(updateSelection('n1'))).toBe('n1');
    expect(selectedUpdate('approval:sess:r1')).toBeNull();
    expect(selectedUpdate(null)).toBeNull();
  });
});

describe('gateQuestion', () => {
  it.each([
    ['a longer question', 'gate', { question: 'Ship it?\n\nCI is green.' }, 'Ship it?\n\nCI is green.'],
    ['the title again', 'gate', { question: '  Ship it? ' }, null],
    ['a blank question', 'gate', { question: '   ' }, null],
    ['no question', 'gate', {}, null],
    ['a question that is not a string', 'gate', { question: ['Ship it?', 'more'] }, null],
    ['not a gate', 'question', { question: 'Ship it?\n\nCI is green.' }, null],
  ] as const)('%s', (_, kind, b, want) => expect(gateQuestion(body(b, { kind, title: 'Ship it?' }))).toBe(want));
});

describe('noticeText', () => {
  it.each([
    ['its text', { text: 'CI failed' }, 'CI failed'],
    ['no text: the title', {}, 'Title'],
    ['text that is not a string: the title', { text: 5 }, 'Title'],
  ])('%s', (_, b, want) => expect(noticeText(body(b, { kind: 'notice' }))).toBe(want));
});

describe('passesOf', () => {
  it('reads each pass, with Agent and nulls for what is missing or of the wrong type', () => {
    const p = body({ passes: [{ persona: 'Reviewer', outcome: 'changes_requested', summary: 'Fix x' }, {}, null, { persona: 1, outcome: 2, summary: [] }] });
    expect(passesOf(p)).toEqual([
      { persona: 'Reviewer', outcome: 'changes_requested', summary: 'Fix x' },
      { persona: 'Agent', outcome: null, summary: null },
      { persona: 'Agent', outcome: null, summary: null },
      { persona: 'Agent', outcome: null, summary: null },
    ]);
  });

  it.each([['missing', {}], ['not a list', { passes: 'two' }]])('passes %s: none', (_, b) => expect(passesOf(body(b))).toEqual([]));
});

describe('resends', () => {
  const resend = [{ id: 'resend', title: 'Resume', primary: true }];
  it.each([
    ['a stalled step with resend', 'stalled', resend, true],
    ['a stalled step without', 'stalled', [{ id: 'nudge', title: 'Nudge', primary: true }], false],
    ['another kind with resend', 'gate', resend, false],
  ] as const)('%s', (_, kind, choices, want) => expect(resends(item({ kind, choices: [...choices] }))).toBe(want));
});

describe('outcomeTone', () => {
  it.each([
    [null, 'neutral'], ['', 'neutral'],
    ['approved', 'good'], ['Approve', 'good'], ['pass', 'good'], ['passed', 'good'], ['merged', 'good'], ['DONE', 'good'],
    ['changes_requested', 'bad'], ['fail', 'bad'], ['rejected', 'bad'], ['blocked', 'bad'], ['error', 'bad'],
    ['submitted', 'neutral'], ['approved_with_notes', 'neutral'],
  ] as const)('%s is %s', (outcome, want) => expect(outcomeTone(outcome)).toBe(want));
});

describe('changeRequestLabel', () => {
  it('names the forge’s word, and nothing without a pull request', () => {
    expect(changeRequestLabel(item({ changeRequest: { name: 'merge request', url: 'u' } }))).toBe('Open merge request');
    expect(changeRequestLabel(item())).toBeNull();
  });
});

describe('itemLabel', () => {
  it.each([
    ['a gate', 'gate', 'gate', 'Gate'],
    ['a gate under Ready to accept', 'gate', 'accept', 'Accept'],
    ['a gate that stalled', 'gate', 'stalled', 'Stalled'],
    ['an approval', 'approval', 'approval', 'Approval'],
    ['a conversation’s own question', 'approval', 'question', 'Question'],
    ['a triaged ticket', 'admit', 'admit', 'Triaged ticket'],
  ] as const)('%s', (_, kind, group, want) => expect(itemLabel(item({ kind, group }))).toBe(want));
});

describe('longTitle', () => {
  it.each([
    ['a one-line question', 'question', 'Which file?', false],
    ['a question over 120 characters', 'question', 'x'.repeat(121), true],
    ['a question of exactly 120', 'question', 'x'.repeat(120), false],
    ['a question of two lines', 'question', 'Which file?\nOr none', true],
    ['a long gate title', 'gate', 'x'.repeat(200), false],
  ] as const)('%s', (_, kind, title, want) => expect(longTitle(item({ kind, title }))).toBe(want));
});
