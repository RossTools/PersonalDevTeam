import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { AgentDot, AgentLabel, agentState, itemState, rollup, type AgentStateInput, type AgentStateValue } from './AgentState';
import type { Session } from '../api/gen/Session';
import type { Run } from '../api/gen/Run';

type Row = [string, AgentStateInput, Omit<AgentStateValue, 'unseen'> & { unseen?: boolean }];

// Spec §6.5, one row per situation (plus the cases the table implies: unknown activity, runs).
const rows: Row[] = [
  ['agent working', { kind: 'agent_tui', status: 'running', activity: 'working' }, { dot: 'running', pulse: true, label: 'Working', tone: 'running' }],
  ['agent needs you', { kind: 'agent_tui', status: 'running', activity: 'needs_you' }, { dot: 'waiting', pulse: false, label: 'Needs you', tone: 'waiting' }],
  ['agent needs you, unseen', { kind: 'agent_tui', status: 'running', activity: 'needs_you', unseen: true }, { dot: 'waiting', pulse: false, label: 'Needs you', tone: 'waiting', unseen: true }],
  ['agent idle, unseen', { kind: 'agent_tui', status: 'running', activity: 'idle', unseen: true }, { dot: 'idle', pulse: false, label: 'Done', tone: 'muted', unseen: true }],
  ['agent idle, seen', { kind: 'agent_tui', status: 'running', activity: 'idle', unseen: false }, { dot: 'idle', pulse: false, label: 'Idle', tone: 'muted' }],
  ['agent activity unknown', { kind: 'agent_tui', status: 'running', activity: 'unknown' }, { dot: 'idle', pulse: false, label: null, tone: 'muted' }],
  ['agent no activity yet', { kind: 'agent_tui', status: 'running', activity: null }, { dot: 'idle', pulse: false, label: null, tone: 'muted' }],
  ['agent ended', { kind: 'agent_tui', status: 'finished', activity: 'working' }, { dot: 'ended', pulse: false, label: 'Ended', tone: 'muted' }],
  ['agent interrupted', { kind: 'agent_tui', status: 'interrupted', activity: 'needs_you' }, { dot: 'ended', pulse: false, label: 'Interrupted', tone: 'muted' }],
  ['shell running', { kind: 'shell', status: 'running', activity: null }, { dot: 'running', pulse: false, label: null, tone: 'running' }],
  ['shell ended', { kind: 'shell', status: 'finished' }, { dot: 'ended', pulse: false, label: 'Ended', tone: 'muted' }],
  ['shell interrupted', { kind: 'shell', status: 'interrupted' }, { dot: 'ended', pulse: false, label: 'Interrupted', tone: 'muted' }],
  ['run setting up', { kind: 'run', status: 'setting_up' }, { dot: 'running', pulse: true, label: null, tone: 'running' }],
  ['run starting', { kind: 'run', status: 'starting' }, { dot: 'running', pulse: true, label: null, tone: 'running' }],
  ['run ready', { kind: 'run', status: 'ready' }, { dot: 'running', pulse: false, label: null, tone: 'running' }],
  ['run running', { kind: 'run', status: 'running' }, { dot: 'running', pulse: false, label: null, tone: 'running' }],
  ['run passed', { kind: 'run', status: 'passed' }, { dot: 'ended', pulse: false, label: 'Done', tone: 'muted' }],
  ['run failed', { kind: 'run', status: 'failed' }, { dot: 'failed', pulse: false, label: 'Failed', tone: 'failed' }],
  ['run stopped', { kind: 'run', status: 'stopped' }, { dot: 'ended', pulse: false, label: 'Ended', tone: 'muted' }],
];

describe('agentState (spec §6.5)', () => {
  it.each(rows)('%s', (_name, input, expected) => {
    expect(agentState(input)).toEqual({ unseen: false, ...expected });
  });
});

describe('itemState', () => {
  it('reads a session or a run', () => {
    const session = { kind: 'agent_tui', status: 'running', activity: 'working' } as Session;
    expect(itemState({ session, unseen: false }).label).toBe('Working');
    const run = { state: 'failed' } as Run;
    expect(itemState({ run, unseen: false }).dot).toBe('failed');
  });
});

const s = (input: AgentStateInput) => agentState(input);
const agent = (activity: 'working' | 'needs_you' | 'idle', unseen = false) => s({ kind: 'agent_tui', status: 'running', activity, unseen });

describe('rollup', () => {
  it('takes the most urgent: needs you > failed > working > done > idle', () => {
    expect(rollup([agent('idle'), agent('working'), agent('needs_you')])?.label).toBe('Needs you');
    expect(rollup([agent('working'), s({ kind: 'run', status: 'failed' })])?.label).toBe('Failed');
    expect(rollup([agent('idle'), agent('working')])?.label).toBe('Working');
    expect(rollup([agent('idle'), agent('idle', true)])?.label).toBe('Done');
    expect(rollup([agent('idle')])?.label).toBe('Idle');
  });

  it('is unseen when any item is', () => {
    const r = rollup([agent('working'), agent('idle', true)]);
    expect(r).toMatchObject({ label: 'Working', unseen: true, pulse: true });
  });

  it('ignores shells and ended items; null when nothing counts', () => {
    expect(rollup([])).toBeNull();
    expect(rollup([s({ kind: 'shell', status: 'running' }), s({ kind: 'agent_tui', status: 'finished' })])).toBeNull();
    expect(rollup([s({ kind: 'agent_tui', status: 'finished', unseen: true })])).toMatchObject({ dot: 'idle', label: null, unseen: true });
  });
});

describe('AgentDot / AgentLabel', () => {
  it('draws one dot named by the label, plus the unseen dot', () => {
    render(<AgentDot state={agent('needs_you', true)} />);
    expect(screen.getByRole('img', { name: 'Needs you' })).toHaveAttribute('data-status', 'waiting');
    expect(screen.getByRole('img', { name: 'unseen' })).toBeInTheDocument();
    expect(screen.getAllByRole('img')).toHaveLength(2);
  });

  it('pulses while working and names a label-less dot by its status', () => {
    render(<><AgentDot state={agent('working')} /><AgentDot state={s({ kind: 'shell', status: 'running' })} /></>);
    expect(screen.getByRole('img', { name: 'Working' })).toHaveAttribute('data-pulse');
    expect(screen.getByRole('img', { name: 'running' })).not.toHaveAttribute('data-pulse');
    expect(screen.queryByRole('img', { name: 'unseen' })).not.toBeInTheDocument();
  });

  it('can leave out the unseen dot', () => {
    render(<AgentDot state={agent('idle', true)} unseenDot={false} />);
    expect(screen.queryByRole('img', { name: 'unseen' })).not.toBeInTheDocument();
  });

  it('labels in the tone of the state, nothing without a label', () => {
    const { container } = render(<><AgentLabel state={agent('needs_you')} /><AgentLabel state={s({ kind: 'shell', status: 'running' })} /></>);
    expect(screen.getByText('Needs you')).toHaveClass('text-waiting');
    expect(container.querySelectorAll('span')).toHaveLength(1);
  });
});
