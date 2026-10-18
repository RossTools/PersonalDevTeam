import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { OverlapBadge } from './OverlapBadge';

describe('OverlapBadge', () => {
  it('renders nothing without overlap', () => {
    const { container } = render(<OverlapBadge level="none" keys={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('is amber for an overlap and names the ticket', () => {
    render(<OverlapBadge level="overlap" keys={['PDT-51']} />);
    const b = screen.getByTitle('Overlaps PDT-51');
    expect(b).toHaveTextContent('Overlaps PDT-51');
    expect(b.className).toContain('bg-waiting-soft');
  });

  it('is red for a conflict and counts the rest', () => {
    render(<OverlapBadge level="conflict" keys={['PDT-51', 'PDT-52', 'PDT-60']} />);
    const b = screen.getByTitle('Conflicts with PDT-51, PDT-52, PDT-60');
    expect(b).toHaveTextContent('Conflicts with PDT-51+2 and 2 more');
    expect(b).toHaveAttribute('title', 'Conflicts with PDT-51, PDT-52, PDT-60');
    expect(b.className).toContain('bg-failed-soft');
  });
});
