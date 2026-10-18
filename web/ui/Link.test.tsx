import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Link } from './Link';

describe('Link', () => {
  it('internal link: href, same tab, no icon', () => {
    render(<Link href="#settings">Settings</Link>);
    const a = screen.getByRole('link', { name: 'Settings' });
    expect(a).toHaveAttribute('href', '#settings');
    expect(a).not.toHaveAttribute('target');
    expect(a.querySelector('svg')).toBeNull();
  });

  it('external link opens a new tab with rel=noreferrer and an icon', () => {
    render(
      <Link href="https://github.com/o/r" external>
        GitHub
      </Link>,
    );
    const a = screen.getByRole('link', { name: 'GitHub' });
    expect(a).toHaveAttribute('target', '_blank');
    expect(a).toHaveAttribute('rel', 'noreferrer');
    expect(a.querySelector('svg')).toHaveAttribute('aria-hidden', 'true');
  });

  it('action link (no href) runs onClick on click and Enter', async () => {
    const onClick = vi.fn();
    render(<Link onClick={onClick}>Open settings</Link>);
    const a = screen.getByRole('link', { name: 'Open settings' });
    fireEvent.click(a);
    await userEvent.click(a);
    a.focus();
    await userEvent.keyboard('{Enter}');
    expect(onClick).toHaveBeenCalledTimes(3);
  });
});
