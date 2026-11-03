import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Clock } from 'lucide-react';
import { Badge } from './Badge';

const badgeOf = (text: string | RegExp) => screen.getByText(text).closest('[data-slot=badge]') as HTMLElement;

describe('Badge', () => {
  it('is neutral by default', () => {
    render(<Badge>draft</Badge>);
    expect(badgeOf('draft').className).toContain('bg-muted');
  });

  it('uses the soft fill for status tones', () => {
    for (const [tone, cls] of [['running', 'bg-running-soft'], ['waiting', 'bg-waiting-soft'], ['failed', 'bg-failed-soft'], ['accent', 'bg-accent']] as const) {
      const { unmount } = render(<Badge tone={tone}>{tone}</Badge>);
      expect(badgeOf(tone).className).toContain(cls);
      unmount();
    }
  });

  it('never grows past its container: a long label truncates its text, icons stay whole', () => {
    const hold = 'overlaps PDT-367 in workflows';
    render(<Badge tone="waiting" title={`Waiting: ${hold}`}><Clock aria-hidden /> Waiting: {hold}</Badge>);
    const text = screen.getByText(`Waiting: ${hold}`);
    expect(text.className).toContain('truncate');
    expect(text.className).toContain('min-w-0');
    const badge = badgeOf(`Waiting: ${hold}`);
    expect(badge.className).toContain('max-w-full');
    expect(badge.className).toContain('min-w-0');
    expect(badge.querySelector(':scope > svg')).not.toBeNull();
  });

  it('count variant shows the number, capped', () => {
    const { rerender } = render(<Badge count={7} aria-label="7 unread" />);
    expect(screen.getByLabelText('7 unread')).toHaveTextContent('7');
    rerender(<Badge count={150} aria-label="many" />);
    expect(screen.getByLabelText('many')).toHaveTextContent('99+');
    expect(screen.getByLabelText('many').className).toContain('rounded-full');
    expect(screen.getByLabelText('many').className).not.toContain('max-w-full');
  });

  it('a solid small count is the accent fill at 16px', () => {
    render(<Badge tone="solid" size="sm" count={3} aria-label="3 waiting" />);
    const badge = screen.getByLabelText('3 waiting');
    expect(badge.className).toContain('bg-primary');
    expect(badge.className).toContain('text-primary-foreground');
    expect(badge.className).toContain('h-4');
    expect(badge.className).toContain('min-w-4');
    expect(badge.className).not.toContain('min-w-5');
  });
});
