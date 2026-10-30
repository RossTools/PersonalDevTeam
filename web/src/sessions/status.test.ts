import { describe, expect, it } from 'vitest';
import { hiddenStepSession } from './status';

describe('hiddenStepSession', () => {
  const persona = { id: 'p', name: 'Triage', version: 1, stepId: 'st' };
  const step = { persona, attemptId: null, ticket: null, status: 'finished' as const };

  it('hides an ended workflow step session with no ticket', () => {
    expect(hiddenStepSession(step)).toBe(true);
    expect(hiddenStepSession({ ...step, status: 'interrupted' })).toBe(true);
  });

  it('keeps running ones, ticket sessions and sessions no step started', () => {
    expect(hiddenStepSession({ ...step, status: 'running' })).toBe(false);
    expect(hiddenStepSession({ ...step, attemptId: 'a', ticket: { id: 't', key: 'PDT-1' } })).toBe(false);
    expect(hiddenStepSession({ ...step, persona: { ...persona, stepId: null } })).toBe(false);
    expect(hiddenStepSession({ ...step, persona: null })).toBe(false);
  });
});
