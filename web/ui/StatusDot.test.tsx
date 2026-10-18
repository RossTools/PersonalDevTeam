import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StatusDot } from './StatusDot';

describe('StatusDot', () => {
  it('labels and colours every status', () => {
    for (const [s, cls] of [['running', 'bg-running'], ['waiting', 'bg-waiting'], ['failed', 'bg-failed'], ['idle', 'bg-idle']] as const) {
      const { unmount } = render(<StatusDot status={s} />);
      expect(screen.getByRole('img', { name: s }).className).toContain(cls);
      unmount();
    }
  });

  it('ended is a hollow ring', () => {
    render(<StatusDot status="ended" />);
    const dot = screen.getByRole('img', { name: 'ended' });
    expect(dot.className).toContain('border-idle');
    expect(dot.className).toContain('bg-transparent');
  });

  it('pulses only when running, never under reduced motion', () => {
    render(<StatusDot status="running" pulse label="Working" />);
    const dot = screen.getByRole('img', { name: 'Working' });
    expect(dot.className).toContain('animate-pulse');
    expect(dot.className).toContain('motion-reduce:animate-none');
  });

  it('ignores pulse on other statuses', () => {
    render(<StatusDot status="waiting" pulse />);
    expect(screen.getByRole('img', { name: 'waiting' }).className).not.toContain('animate-pulse');
  });
});
