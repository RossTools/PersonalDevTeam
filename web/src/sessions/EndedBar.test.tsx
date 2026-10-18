import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { EndedBar } from './EndedBar';

describe('EndedBar', () => {
  it('offers Restart and Remove… for an ended session', async () => {
    const onRestart = vi.fn();
    const onRemove = vi.fn();
    render(<EndedBar status="finished" onRestart={onRestart} onRemove={onRemove} />);
    expect(screen.getByRole('group', { name: 'Session ended' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await userEvent.click(screen.getByRole('button', { name: 'Remove…' }));
    expect(onRestart).toHaveBeenCalledOnce();
    expect(onRemove).toHaveBeenCalledOnce();
  });

  it('offers Resume for an interrupted session, explaining it for agents only', () => {
    const { rerender } = render(<EndedBar status="interrupted" agent onRestart={() => {}} onRemove={() => {}} />);
    expect(screen.getByRole('group', { name: 'Interrupted when the daemon stopped' })).toHaveTextContent('the agent resumes its last conversation');
    expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Remove…' })).not.toBeInTheDocument();
    rerender(<EndedBar status="interrupted" onRestart={() => {}} />);
    expect(screen.queryByText(/resumes its last conversation/)).not.toBeInTheDocument();
  });
});
