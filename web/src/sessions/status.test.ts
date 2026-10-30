import { describe, expect, it } from 'vitest';
import { startedByStep } from './status';

describe('startedByStep', () => {
  const persona = { id: 'p', name: 'Coder', version: 1, stepId: 'st' };

  it('is a session a workflow step started, whatever its ticket or status', () => {
    expect(startedByStep({ persona })).toBe(true);
    expect(startedByStep({ persona: { ...persona, name: 'Triage' } })).toBe(true);
  });

  it('is not a session the user started, from a ticket or not', () => {
    expect(startedByStep({ persona: { ...persona, stepId: null } })).toBe(false);
    expect(startedByStep({ persona: null })).toBe(false);
  });
});
