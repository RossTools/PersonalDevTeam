import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ToolCard } from './ToolCard';

describe('ToolCard', () => {
  it('is a named group, not a landmark, with its heading, aside and rows', () => {
    render(
      <ToolCard icon={<svg />} aria-label="Changed ALP-2" heading="Changed ALP-2" aside={<span>Queued</span>}>
        <div>Stage</div>
        <div>Priority</div>
      </ToolCard>,
    );
    const card = screen.getByRole('group', { name: 'Changed ALP-2' });
    expect(card).toHaveTextContent('Changed ALP-2QueuedStagePriority');
    expect(screen.queryByRole('region')).not.toBeInTheDocument();
    expect(card).not.toHaveAttribute('tabindex');
  });

  it('takes focus when given a ref', () => {
    const ref = { current: null as HTMLDivElement | null };
    render(<ToolCard ref={ref} icon={<svg />} aria-label="Watching" heading="Watching" />);
    ref.current?.focus();
    expect(screen.getByRole('group', { name: 'Watching' })).toHaveFocus();
  });
});
