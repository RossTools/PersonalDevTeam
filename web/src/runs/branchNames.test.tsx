import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BranchNamesView } from '../api/gen/BranchNamesView';
import type { Project } from '../api/gen/Project';
import { renderWithQuery, stubApi } from '../test-utils';
import { BranchNames } from './BranchNames';

afterEach(() => {
  vi.unstubAllGlobals();
});

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', forge: null, kind: 'repo' };
const view = (over: Partial<BranchNamesView> = {}): BranchNamesView => ({
  setting: { template: null, types: {} },
  template: 'pdt-{key}-{slug}',
  types: { bug: 'fix' },
  source: 'default',
  fileError: null,
  slash: false,
  previews: [
    { label: 'A bug with a GitHub issue and a Jira issue', name: 'pdt-PDT-48-fix-the-login-crash' },
    { label: 'PDT-3 Dark mode', name: 'pdt-PDT-3-dark-mode' },
  ],
  parts: [{ name: 'key', description: "The ticket's key: PDT-48" }, { name: 'jira', description: "The linked Jira issue's key: ABC-123" }],
  ...over,
});

describe('Branch names', () => {
  it('previews a template as it is typed, warns about slashes and saves it', async () => {
    const calls = stubApi({
      'GET /api/projects/p/branch-names': view(),
      'POST /api/projects/p/branch-names/preview': {
        error: null,
        slash: true,
        previews: [{ label: 'A bug with a GitHub issue and a Jira issue', name: 'fix/ABC-123-fix-the-login-crash' }],
      },
      'PUT /api/projects/p/branch-names': view({ setting: { template: '{type}/{jira|key}-{slug}', types: { docs: 'docs' } }, source: 'setting', slash: true }),
    });
    renderWithQuery(<BranchNames project={project} />);
    const previews = await screen.findByRole('list', { name: 'Branch name previews' });
    expect(within(previews).getByText('pdt-PDT-3-dark-mode')).toBeInTheDocument();
    expect(screen.getByText('{jira}')).toHaveAttribute('title', "The linked Jira issue's key: ABC-123");

    await userEvent.type(screen.getByRole('textbox', { name: 'Branch name template' }), '{{type}/{{jira|key}-{{slug}');
    await userEvent.type(screen.getByRole('textbox', { name: 'Branch types' }), 'docs=docs');
    expect(await screen.findByText('fix/ABC-123-fix-the-login-crash')).toBeInTheDocument();
    expect(screen.getByText(/some tools handle them badly/)).toBeInTheDocument();
    await waitFor(() => {
      const last = calls.filter((c) => c.method === 'POST').at(-1);
      expect(last?.body).toEqual({ template: '{type}/{jira|key}-{slug}', types: { docs: 'docs' } });
    });

    await userEvent.click(screen.getByRole('button', { name: 'Save branch names' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ template: '{type}/{jira|key}-{slug}', types: { docs: 'docs' } }));
  });

  it('shows what is wrong with a template', async () => {
    stubApi({
      'GET /api/projects/p/branch-names': view(),
      'POST /api/projects/p/branch-names/preview': { error: '{nope} is not a part', slash: false, previews: [] },
    });
    renderWithQuery(<BranchNames project={project} />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Branch name template' }), '{{nope}');
    expect(await screen.findByText('{nope} is not a part')).toBeInTheDocument();
    expect(screen.queryByRole('list', { name: 'Branch name previews' })).not.toBeInTheDocument();
  });

  it('shows the repository file’s template read-only, with its problem', async () => {
    stubApi({
      'GET /api/projects/p/branch-names': view({ source: 'file', template: '{nope}', fileError: '[branch] in .pdt/project.toml: {nope} is not a part' }),
    });
    renderWithQuery(<BranchNames project={project} />);
    expect(await screen.findByText('Set by .pdt/project.toml, which wins over this setting.')).toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: 'Branch name template' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Save branch names' })).not.toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent('{nope} is not a part Branches get the default names meanwhile.');
  });
});
