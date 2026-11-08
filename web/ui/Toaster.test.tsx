import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Toaster, toast } from './Toaster';

afterEach(() => act(() => { toast.dismiss(); }));

describe('toast', () => {
  it('shows success, error and info messages', async () => {
    render(<Toaster />);
    act(() => {
      toast.success('Saved');
      toast.error('Could not stop the run');
      toast.info('Branch pushed');
    });
    expect(await screen.findByText('Saved')).toBeInTheDocument();
    expect(screen.getByText('Could not stop the run')).toBeInTheDocument();
    expect(screen.getByText('Branch pushed')).toBeInTheDocument();
  });

  it('runs the action', async () => {
    const onClick = vi.fn();
    render(<Toaster />);
    act(() => { toast.error('Push failed', { action: { label: 'Retry', onClick } }); });
    await userEvent.click(await screen.findByRole('button', { name: 'Retry' }));
    expect(onClick).toHaveBeenCalled();
  });

  it('sits on the toast layer', async () => {
    const { container } = render(<Toaster />);
    act(() => { toast.info('Hello'); });
    await screen.findByText('Hello');
    const ol = container.ownerDocument.querySelector<HTMLElement>('[data-sonner-toaster]')!;
    expect(ol.style.zIndex).toBe('var(--pdt-z-toast)');
  });

  it('sits above the bars at the bottom of a view on a desktop', async () => {
    const { container } = render(<Toaster />);
    act(() => { toast.info('Hello'); });
    await screen.findByText('Hello');
    const ol = container.ownerDocument.querySelector<HTMLElement>('[data-sonner-toaster]')!;
    expect(ol.style.getPropertyValue('--offset-bottom')).toBe('8rem');
  });
});
