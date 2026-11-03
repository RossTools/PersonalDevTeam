import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { DropZone, useFileDrop, type FileDrag } from './DropZone';

const file = new File(['# Plan'], 'Plan.md', { type: 'text/markdown' });
/** A drag of files, as the browser gives it: its types, its items and (on drop) its files. */
const files = (dropped: File[] = []) => ({
  dataTransfer: { types: ['Files'], items: [{ kind: 'file', type: 'text/markdown' }], files: dropped, dropEffect: 'none' },
});
const describeDrag = (d: FileDrag) => `Drop to import ${d.name ?? 'it'} (${d.type})`;

describe('DropZone', () => {
  it('says what to drop, and chooses a file with its button', async () => {
    const onFile = vi.fn();
    render(<DropZone onFile={onFile} accept=".md">Drop a .md</DropZone>);
    expect(screen.getByText('Drop a .md')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Choose file…' })).toBeInTheDocument();
    await userEvent.upload(screen.getByLabelText('File to import'), file);
    expect(onFile).toHaveBeenCalledWith(file);
  });

  it('highlights a file dragged over it, says what dropping does, and takes the drop', () => {
    const onFile = vi.fn();
    const { container } = render(<DropZone onFile={onFile} describe={describeDrag}>Drop a .md</DropZone>);
    const zone = container.querySelector('[data-slot="drop-zone"]')!;
    fireEvent.dragEnter(zone, files());
    expect(zone).toHaveAttribute('data-dragging');
    expect(screen.getByText('Drop to import it (text/markdown)')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Choose file…' })).toBeNull();
    fireEvent.drop(zone, files([file]));
    expect(onFile).toHaveBeenCalledWith(file);
    expect(zone).not.toHaveAttribute('data-dragging');
    expect(screen.getByText('Drop a .md')).toBeInTheDocument();
  });

  it('ends the drag only when it leaves the zone, not a child', () => {
    const { container } = render(<DropZone onFile={() => {}}>Drop a .md</DropZone>);
    const zone = container.querySelector('[data-slot="drop-zone"]')!;
    fireEvent.dragEnter(zone, files());
    fireEvent.dragEnter(screen.getByText('Drop to import it'), files());
    fireEvent.dragLeave(zone, files());
    expect(zone).toHaveAttribute('data-dragging');
    fireEvent.dragLeave(zone, files());
    expect(zone).not.toHaveAttribute('data-dragging');
  });

  it('leaves drags of anything but files alone, and takes nothing while pending', () => {
    const onFile = vi.fn();
    const { container, rerender } = render(<DropZone onFile={onFile}>Drop a .md</DropZone>);
    const zone = container.querySelector('[data-slot="drop-zone"]')!;
    fireEvent.dragEnter(zone, { dataTransfer: { types: ['text/plain'], items: [], files: [] } });
    expect(zone).not.toHaveAttribute('data-dragging');
    rerender(<DropZone onFile={onFile} pending>Drop a .md</DropZone>);
    fireEvent.drop(zone, files([file]));
    expect(onFile).not.toHaveBeenCalled();
  });

  it('shows the drag of a larger area given as drop, whose drop lands here', () => {
    const onFile = vi.fn();
    function Area() {
      const drop = useFileDrop(onFile);
      return (
        <div data-testid="area" {...drop.dropProps}>
          <p>Elsewhere</p>
          <DropZone onFile={onFile} drop={drop} describe={describeDrag}>Drop a .md</DropZone>
        </div>
      );
    }
    render(<Area />);
    fireEvent.dragEnter(screen.getByText('Elsewhere'), files());
    expect(screen.getByText('Drop to import it (text/markdown)')).toBeInTheDocument();
    fireEvent.drop(screen.getByText('Elsewhere'), files([file]));
    expect(onFile).toHaveBeenCalledTimes(1);
  });
});
