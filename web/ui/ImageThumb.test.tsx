import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ImageThumb } from './ImageThumb';

describe('ImageThumb', () => {
  it('shows the image, its upload and a remove button', async () => {
    const onRemove = vi.fn();
    render(<ImageThumb src="blob:x" alt="shot.png" pending onRemove={onRemove} />);
    expect(screen.getByRole('img', { name: 'shot.png' })).toHaveAttribute('src', 'blob:x');
    expect(screen.getByRole('status', { name: 'Uploading' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Remove shot.png' }));
    expect(onRemove).toHaveBeenCalled();
  });

  it('opens a sent image and has a placeholder when there is nothing to show', () => {
    const { rerender } = render(<ImageThumb src="/a.png" href="/a.png" alt="Image 1" />);
    expect(screen.getByRole('link')).toHaveAttribute('href', '/a.png');
    rerender(<ImageThumb alt="Image 2" />);
    expect(screen.getByRole('img', { name: 'Image 2' }).tagName).toBe('SPAN');
  });
});
