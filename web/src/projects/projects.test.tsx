import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddProjectForm, folderName } from './AddProjectForm';

afterEach(() => vi.unstubAllGlobals());

function wrap(ui: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

describe('projects', () => {
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

  it('names a project from a typed path when Name is left empty', async () => {
    const fetch = vi.fn().mockImplementation(async () => new Response(
      JSON.stringify({ code: 'not_a_git_repo', message: 'nope' }), { status: 400 },
    ));
    vi.stubGlobal('fetch', fetch);
    wrap(<AddProjectForm />);
    await userEvent.type(screen.getByLabelText('Repository path'), '/tmp/my-repo/');
    expect(screen.getByLabelText('Name')).toHaveAttribute('placeholder', 'my-repo');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    await screen.findByRole('alert');
    const [, init] = fetch.mock.calls.find(([u]) => String(u).endsWith('/api/projects'))!;
    expect(JSON.parse(init.body)).toMatchObject({ name: 'my-repo', repoPath: '/tmp/my-repo/' });
  });

  it('takes the last folder of a path', () => {
    expect(folderName('/a/b')).toBe('b');
    expect(folderName('C:\\work\\repo\\')).toBe('repo');
    expect(folderName('~/x')).toBe('x');
    expect(folderName('  ')).toBe('');
  });
});
