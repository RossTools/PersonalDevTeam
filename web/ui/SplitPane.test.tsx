import { describe, expect, it, beforeEach } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { SplitPane } from './SplitPane';

describe('SplitPane', () => {
  beforeEach(() => localStorage.clear());

  it('shows both panes and resizes with the keyboard, remembering the size', () => {
    const { unmount } = render(<SplitPane id="t" start={<p>Files</p>} end={<p>Diff</p>} defaultSize={200} min={100} max={400} label="Resize files" />);
    expect(screen.getByText('Files')).toBeInTheDocument();
    expect(screen.getByText('Diff')).toBeInTheDocument();
    const sep = screen.getByRole('separator', { name: 'Resize files' });
    expect(sep).toHaveAttribute('aria-valuenow', '200');
    fireEvent.keyDown(sep, { key: 'ArrowRight' });
    expect(sep).toHaveAttribute('aria-valuenow', '216');
    fireEvent.keyDown(sep, { key: 'End' });
    expect(sep).toHaveAttribute('aria-valuenow', '400');
    fireEvent.keyDown(sep, { key: 'Home' });
    expect(sep).toHaveAttribute('aria-valuenow', '100');
    fireEvent.keyDown(sep, { key: 'ArrowLeft' });
    expect(sep).toHaveAttribute('aria-valuenow', '100');
    unmount();
    render(<SplitPane id="t" start={null} end={null} defaultSize={200} min={100} max={400} />);
    expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '100');
  });

  it('resizes by dragging the divider', () => {
    render(<SplitPane id="d" start={null} end={null} defaultSize={200} />);
    const sep = screen.getByRole('separator');
    fireEvent.pointerDown(sep, { clientX: 300, pointerId: 1 });
    fireEvent.pointerMove(sep, { clientX: 350, pointerId: 1 });
    fireEvent.pointerUp(sep, { pointerId: 1 });
    expect(sep).toHaveAttribute('aria-valuenow', '250');
  });
});
