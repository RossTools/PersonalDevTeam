import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProjectsList } from './ProjectsList';
import { AddProjectForm } from './AddProjectForm';

afterEach(() => vi.unstubAllGlobals());

function wrap(ui: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

describe('projects', () => {
  it('lists projects from the API', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response(
      JSON.stringify([{ id: '1', name: 'PDT', repoPath: 'C:\\work\\pdt', baseBranch: 'main', createdAt: 1 }]),
      { status: 200 },
    )));
    wrap(<ProjectsList selectedId={null} onSelect={() => {}} />);
    expect(await screen.findByText('PDT')).toBeInTheDocument();
  });

  it('shows the server error message when adding fails', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => new Response(
      JSON.stringify({ code: 'not_a_git_repo', message: 'not a git repository: C:\\tmp' }), { status: 400 },
    )));
    wrap(<AddProjectForm />);
    await userEvent.type(screen.getByLabelText('Name'), 'x');
    await userEvent.type(screen.getByLabelText('Repository path'), 'C:\\tmp');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('not a git repository: C:\\tmp');
  });
});
