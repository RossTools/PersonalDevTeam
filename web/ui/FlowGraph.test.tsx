import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { FlowGraph, type FlowNode } from './FlowGraph';

const nodes: FlowNode[] = [
  { id: 'ci', title: 'CI failed', subtitle: 'ci.run_failed', input: false, ports: [{ id: 'out', label: 'Then' }], position: { x: 0, y: 0 } },
  {
    id: 'ask', title: 'Ask', subtitle: 'gate', ports: [{ id: 'yes', label: 'yes' }, { id: 'no', label: 'no' }], position: { x: 300, y: 0 },
    status: 'waiting', issue: { severity: 'error', messages: ['ask needs a choice'] },
  },
];

describe('FlowGraph', () => {
  it('draws each node with its ports, status and issues, and selects one on click', () => {
    const onSelect = vi.fn();
    render(<div style={{ width: 800, height: 600 }}><FlowGraph aria-label="Graph" nodes={nodes} edges={[{ from: 'ci', port: 'out', to: 'ask', taken: true }]} onSelect={onSelect} /></div>);
    expect(screen.getByText('CI failed')).toBeInTheDocument();
    expect(screen.getByText('ci.run_failed')).toBeInTheDocument();
    expect(screen.getByText('yes')).toBeInTheDocument();
    expect(screen.getByText('no')).toBeInTheDocument();
    expect(screen.getByTestId('flow-node-ask')).toHaveAttribute('data-status', 'waiting');
    expect(screen.getByLabelText('Error: ask needs a choice')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Ask'));
    expect(onSelect).toHaveBeenCalledWith({ kind: 'node', id: 'ask' });
  });
});
