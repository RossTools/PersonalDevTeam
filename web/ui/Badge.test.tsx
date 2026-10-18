import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Badge } from './Badge';

describe('Badge', () => {
  it('is neutral by default', () => {
    render(<Badge>draft</Badge>);
    expect(screen.getByText('draft').className).toContain('bg-muted');
  });

  it('uses the soft fill for status tones', () => {
    for (const [tone, cls] of [['running', 'bg-running-soft'], ['waiting', 'bg-waiting-soft'], ['failed', 'bg-failed-soft'], ['accent', 'bg-accent']] as const) {
      const { unmount } = render(<Badge tone={tone}>{tone}</Badge>);
      expect(screen.getByText(tone).className).toContain(cls);
      unmount();
    }
  });

  it('count variant shows the number, capped', () => {
    const { rerender } = render(<Badge count={7} aria-label="7 unread" />);
    expect(screen.getByLabelText('7 unread')).toHaveTextContent('7');
    rerender(<Badge count={150} aria-label="many" />);
    expect(screen.getByLabelText('many')).toHaveTextContent('99+');
    expect(screen.getByLabelText('many').className).toContain('rounded-full');
  });
});
