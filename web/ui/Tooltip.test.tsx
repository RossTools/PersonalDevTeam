import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Button } from './Button';
import { Tooltip } from './Tooltip';

afterEach(() => vi.useRealTimers());

describe('Tooltip', () => {
  it('opens on keyboard focus, describes the trigger, closes on Escape and blur', async () => {
    render(
      <>
        <Tooltip content="Runs the tests">
          <Button>Test</Button>
        </Tooltip>
        <Button>Other</Button>
      </>,
    );
    await userEvent.tab();
    const tip = await screen.findByRole('tooltip');
    expect(tip).toHaveTextContent('Runs the tests');
    expect(screen.getByRole('button', { name: 'Test' })).toHaveAttribute('aria-describedby', tip.id);
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('tooltip')).toBeNull();
    await userEvent.tab({ shift: true });
    await userEvent.tab();
    expect(await screen.findByRole('tooltip')).toBeInTheDocument();
    await userEvent.tab();
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('opens on hover', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(
      <Tooltip content="Later" delay={300}>
        <Button>Hover me</Button>
      </Tooltip>,
    );
    await user.click(document.body); // React Aria shows hover tooltips only when the pointer is the current modality
    await user.hover(screen.getByRole('button', { name: 'Hover me' }));
    act(() => vi.advanceTimersByTime(400));
    expect(screen.getByRole('tooltip')).toHaveTextContent('Later');
  });

  it('wraps a non-@ui focusable trigger with `wrap`', async () => {
    render(
      <Tooltip content="Full branch name" wrap>
        <a href="#b">feature/…</a>
      </Tooltip>,
    );
    await userEvent.tab();
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Full branch name');
  });
});
