import { describe, expect, it } from 'vitest';
import { byDefault, choiceFor, type Choice } from './newSession';

const ALL: Choice[] = ['Claude', 'Claude (terminal)', 'Codex', 'Codex (terminal)', 'Devin', 'Devin (terminal)', 'Shell'];
const id = (c: Choice) => c;

describe('byDefault', () => {
  it('keeps the order for a Claude conversation', () => {
    expect(byDefault(ALL, id, { agent: 'claude', mode: 'conversation' })).toEqual(ALL);
  });

  it('puts the default agent first, each agent in the default view first, Shell last', () => {
    expect(byDefault(ALL, id, { agent: 'devin', mode: 'terminal' })).toEqual([
      'Devin (terminal)', 'Devin', 'Claude (terminal)', 'Claude', 'Codex (terminal)', 'Codex', 'Shell',
    ]);
  });

  it('orders a subset (agents the host lacks left out)', () => {
    expect(byDefault(['Claude', 'Codex', 'Shell'] as Choice[], id, { agent: 'codex', mode: 'conversation' })).toEqual(['Codex', 'Claude', 'Shell']);
  });
});

describe('choiceFor', () => {
  it('names the conversation or the terminal', () => {
    expect(choiceFor('codex', 'conversation')).toBe('Codex');
    expect(choiceFor('devin', 'terminal')).toBe('Devin (terminal)');
  });
});
