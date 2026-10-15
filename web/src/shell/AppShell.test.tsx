import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AppShell } from './AppShell';

function renderShell() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}><AppShell /></QueryClientProvider>);
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response('[]', { status: 200 })));
});

describe('AppShell', () => {
  it('renders the five rail views and marks the active one', async () => {
    renderShell();
    for (const name of ['Inbox', 'Board', 'Sessions', 'Personas', 'Files']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Board' }));
    expect(screen.getByRole('button', { name: 'Board' })).toHaveAttribute('aria-current', 'page');
  });

  it('toggles the bottom panel with Ctrl+`', async () => {
    renderShell();
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
  });

  it('shows the selected project name, not its id', async () => {
    const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response(JSON.stringify([project]), { status: 200 })));
    renderShell();
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    const main = screen.getByRole('main');
    expect(main).toHaveTextContent('Project Alpha');
    expect(main).not.toHaveTextContent(project.id);
  });
});
