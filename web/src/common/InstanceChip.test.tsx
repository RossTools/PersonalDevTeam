import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { InstanceChip } from './InstanceChip';

describe('InstanceChip', () => {
  it('names the instance, and says it is offline with when it was last seen', () => {
    const { rerender } = render(<InstanceChip instance={{ id: 'w', name: 'win-box', lastSeenAt: Date.now() - 1000 }} />);
    expect(screen.getByText('on win-box').closest('[title]')).toHaveAttribute('title', 'Runs on win-box · last seen just now');
    rerender(<InstanceChip instance={{ id: 'w', name: 'win-box', lastSeenAt: Date.now() - 5 * 60_000 }} />);
    expect(screen.getByText('on win-box · offline').closest('[title]')).toHaveAttribute('title', 'win-box is offline · last seen 5m ago');
    rerender(<InstanceChip instance={null} />);
    expect(screen.queryByText(/win-box/)).toBeNull();
  });
});
