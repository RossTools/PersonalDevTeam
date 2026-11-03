import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ListHeading, SectionLabel } from './SectionLabel';

describe('SectionLabel', () => {
  it('is a small uppercase heading, h3 unless told otherwise', () => {
    render(<><SectionLabel>Links</SectionLabel><SectionLabel as="h4">Settings</SectionLabel></>);
    const links = screen.getByRole('heading', { name: 'Links', level: 3 });
    expect(links.className).toContain('uppercase');
    expect(links.className).toContain('text-2xs');
    expect(screen.getByRole('heading', { name: 'Settings', level: 4 })).toBeInTheDocument();
  });

  it('a list heading is inset like the rows and holds trailing controls', () => {
    render(<ListHeading trailing={<button type="button">New</button>}>Notes</ListHeading>);
    const h = screen.getByRole('heading', { level: 2, name: 'Notes' });
    expect(h.parentElement!.className).toContain('px-3');
    expect(screen.getByRole('button', { name: 'New' })).toBeInTheDocument();
  });
});
