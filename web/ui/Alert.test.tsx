import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Alert } from './Alert';

describe('Alert', () => {
  it('errors are announced with role=alert', () => {
    render(<Alert tone="error" title="Could not add the project">Not a git repository.</Alert>);
    const a = screen.getByRole('alert');
    expect(a).toHaveTextContent('Could not add the project');
    expect(a).toHaveTextContent('Not a git repository.');
    expect(a.className).toContain('bg-failed-soft');
  });

  it('warning and info are status, not alert', () => {
    render(
      <>
        <Alert tone="warning">Uncommitted changes</Alert>
        <Alert>FYI</Alert>
      </>,
    );
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getAllByRole('status')).toHaveLength(2);
  });

  it('renders its action', async () => {
    const onClick = vi.fn();
    render(<Alert tone="error" action={<button onClick={onClick}>Retry</button>}>Failed</Alert>);
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onClick).toHaveBeenCalled();
  });
});
