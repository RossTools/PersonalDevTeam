import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { WatchMark } from './WatchMark';

describe('WatchMark', () => {
  it('names who watches, and shows a count when given', () => {
    const { rerender } = render(<WatchMark title="Watched by you" />);
    const mark = screen.getByRole('img', { name: 'Watched by you' });
    expect(mark).toHaveAttribute('title', 'Watched by you');
    expect(mark).toHaveTextContent('');
    rerender(<WatchMark title="Watching 3" count={3} />);
    expect(screen.getByRole('img', { name: 'Watching 3' })).toHaveTextContent('3');
  });
});
