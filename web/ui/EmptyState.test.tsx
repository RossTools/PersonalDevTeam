import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Inbox } from 'lucide-react';
import { EmptyState } from './EmptyState';

describe('EmptyState', () => {
  it('shows the icon, sentence and action', () => {
    const { container } = render(
      <EmptyState icon={<Inbox />} action={<button>Start an agent</button>}>
        No agents running.
      </EmptyState>,
    );
    expect(screen.getByText('No agents running.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start an agent' })).toBeInTheDocument();
    expect(container.querySelector('[data-slot="empty-icon"] svg')).not.toBeNull();
  });

  it('works without icon or action', () => {
    const { container } = render(<EmptyState role="alert">Could not load.</EmptyState>);
    expect(screen.getByRole('alert')).toHaveTextContent('Could not load.');
    expect(container.querySelector('[data-slot="empty-icon"]')).toBeNull();
  });
});
