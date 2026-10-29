import { describe, expect, it } from 'vitest';
import type { AcpAgentView } from '../api/gen/AcpAgentView';
import { agentOf, byDefault, choiceFor, choiceText, newSession, type Choice } from './newSession';

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

  it('gives an ACP agent its conversation, whatever the view', () => {
    expect(choiceFor('acp:fake', 'terminal')).toBe('acp:fake');
  });
});

describe('ACP agents', () => {
  it('come after the built-in agents unless one is the default, and start conversations', () => {
    const all: Choice[] = ['Claude', 'acp:fake', 'Codex', 'Shell'];
    expect(byDefault(all, id, { agent: 'claude', mode: 'conversation' })).toEqual(['Claude', 'Codex', 'acp:fake', 'Shell']);
    expect(byDefault(all, id, { agent: 'acp:fake', mode: 'terminal' })).toEqual(['acp:fake', 'Claude', 'Codex', 'Shell']);
    expect(agentOf('acp:fake')).toBe('acp:fake');
    expect(newSession('p', null, 'acp:fake')).toMatchObject({ kind: 'agent_tui', agent: 'acp:fake', mode: 'conversation' });
    expect(choiceText('acp:fake', [{ agent: 'acp:fake', label: 'Fake' } as unknown as AcpAgentView])).toBe('Fake');
    expect(choiceText('acp:gone', [])).toBe('gone');
  });
});
