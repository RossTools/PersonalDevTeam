import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { MessageBubble } from './MessageBubble';

describe('MessageBubble', () => {
  it('keeps the text as written, with an optional label and meta', () => {
    const { container } = render(<MessageBubble tone="pending" label="Queued" meta="From the terminal">{'a\n  b'}</MessageBubble>);
    expect(screen.getByText('Queued')).toBeInTheDocument();
    expect(screen.getByText('From the terminal')).toBeInTheDocument();
    expect(container.firstElementChild).toHaveAttribute('data-tone', 'pending');
    expect(container.querySelector('.whitespace-pre-wrap')?.textContent).toContain('a\n  b');
  });
});
