import { describe, expect, it } from 'vitest';
import { hiddenStepSession, startedByStep } from './status';

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

describe('hiddenStepSession', () => {
  const persona = { id: 'p', name: 'Triage', version: 1, stepId: 'st' };
  const step = { persona, attemptId: null, ticket: null, status: 'finished' as const };

  it('hides an ended workflow step session with no ticket', () => {
    expect(hiddenStepSession(step)).toBe(true);
    expect(hiddenStepSession({ ...step, status: 'interrupted' })).toBe(true);
  });

  it('hides a ticket\'s workflow step session, running or ended', () => {
    const coder = { ...step, persona: { ...persona, name: 'Coder' }, attemptId: 'a', ticket: { id: 't', key: 'PDT-1' } };
    expect(hiddenStepSession({ ...coder, status: 'running' })).toBe(true);
    expect(hiddenStepSession(coder)).toBe(true);
    expect(hiddenStepSession({ ...coder, status: 'interrupted' })).toBe(true);
    expect(hiddenStepSession({ ...coder, attemptId: null, status: 'running' })).toBe(true);
  });

  it('keeps running ones with no ticket and sessions no step started', () => {
    expect(hiddenStepSession({ ...step, status: 'running' })).toBe(false);
    const started = { ...step, persona: { ...persona, name: 'Coder', stepId: null }, attemptId: 'a', ticket: { id: 't', key: 'PDT-1' } };
    expect(hiddenStepSession({ ...started, status: 'running' })).toBe(false);
    expect(hiddenStepSession({ ...started, persona: null })).toBe(false);
    expect(hiddenStepSession({ ...step, persona: { ...persona, stepId: null } })).toBe(false);
    expect(hiddenStepSession({ ...step, persona: null })).toBe(false);
  });
});
