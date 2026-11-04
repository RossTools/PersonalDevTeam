import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { FlowGraph, openingViewport, type FlowNode } from './FlowGraph';

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

  describe('openingViewport', () => {
    it('centres a graph that fits, at no more than 100%', () => {
      expect(openingViewport({ x: 0, y: 0, width: 200, height: 100 }, { width: 800, height: 600 })).toEqual({ x: 300, y: 250, zoom: 1 });
    });

    it('shrinks a graph to fit while its titles stay readable', () => {
      const v = openingViewport({ x: 0, y: 0, width: 880, height: 100 }, { width: 800, height: 600 });
      expect(v.zoom).toBeCloseTo(720 / 880);
      expect(v.x).toBeCloseTo(40);
    });

    it('opens a wide graph at the minimum zoom, pinned left and centred vertically', () => {
      // Six nodes in a row next to the inspector: the case that used to open at a zoom of about 0.4.
      expect(openingViewport({ x: 100, y: -50, width: 1700, height: 200 }, { width: 740, height: 820 }))
        .toEqual({ x: 40 - 75, y: (820 - 150) / 2 + 37.5, zoom: 0.75 });
    });

    it('pins a tall graph to the top', () => {
      const v = openingViewport({ x: 0, y: 0, width: 200, height: 2000 }, { width: 800, height: 600 });
      expect(v).toEqual({ x: (800 - 150) / 2, y: 40, zoom: 0.75 });
    });

    it('opens an L-shaped graph on its start node, not the bounds\' empty corner', () => {
      // Cut a release: the trigger at (-520, 0), branches reaching up to y -1600 and right to x 2080.
      const bounds = { x: -520, y: -1600, width: 2600, height: 1700 };
      const trigger = { x: -520, y: 0, width: 176, height: 60 };
      const canvas = { width: 580, height: 700 };
      const v = openingViewport(bounds, canvas, trigger);
      expect(v.zoom).toBe(0.75);
      // The trigger sits at the left padding, and as near the middle as the bounds' bottom allows.
      expect(v.x + trigger.x * v.zoom).toBeCloseTo(40);
      const top = v.y + trigger.y * v.zoom;
      expect(top).toBeGreaterThanOrEqual(0);
      expect(top + trigger.height * v.zoom).toBeLessThanOrEqual(canvas.height);
      // No more than the padding below the graph's bottom edge.
      expect(v.y + (bounds.y + bounds.height) * v.zoom).toBeCloseTo(canvas.height - 40);
    });

    it('centres a tall graph on its start node when the bounds allow', () => {
      const v = openingViewport({ x: 0, y: -1000, width: 200, height: 2000 }, { width: 800, height: 600 }, { x: 0, y: 0, width: 200, height: 60 });
      expect(v.y + 30 * v.zoom).toBeCloseTo(300);
      expect(v.x).toBeCloseTo((800 - 150) / 2);
    });
  });
});
