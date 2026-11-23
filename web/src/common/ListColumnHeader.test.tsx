import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ListColumnHeader } from './ListColumnHeader';

const stubPhone = () =>
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('max-width'),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));

afterEach(() => vi.unstubAllGlobals());

describe('ListColumnHeader', () => {
  it('shows the title, the extra buttons, then + New named for what it makes', async () => {
    const onNew = vi.fn();
    render(<ListColumnHeader title="Notes" newLabel="New note" onNew={onNew} extra={<button type="button">Import</button>} />);
    expect(screen.getByRole('heading', { name: 'Notes' })).toBeInTheDocument();
    const buttons = screen.getAllByRole('button');
    expect(buttons.map((b) => b.textContent?.trim())).toEqual(['Import', 'New']);
    await userEvent.click(screen.getByRole('button', { name: 'New note' }));
    expect(onNew).toHaveBeenCalled();
  });

  it('is only a title without New', () => {
    render(<ListColumnHeader title="Files" />);
    expect(screen.getByRole('heading', { name: 'Files' })).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('leaves the title to the top bar on a phone, and is left out when it has no buttons', () => {
    stubPhone();
    const { container, rerender } = render(<ListColumnHeader title="Notes" newLabel="New note" onNew={() => {}} />);
    expect(screen.queryByRole('heading')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'New note' })).toBeInTheDocument();
    rerender(<ListColumnHeader title="Personas" />);
    expect(container).toBeEmptyDOMElement();
  });
});
