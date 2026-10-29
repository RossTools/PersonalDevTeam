import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Connection } from '../api/gen/Connection';
import type { Project } from '../api/gen/Project';
import { ConnectionDialog } from '../connections/ConnectionDialog';
import { ProjectSettingsPanel } from '../runs/ProjectSettingsPanel';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { OpenTicketContext } from './actions';
import { ExternalKeys } from './ExternalKeys';
import { ImportJiraDialog } from './ImportJiraDialog';
import { CiBadge } from './PullRequest';

vi.mock('../files/CodeViewer', () => ({ CodeViewer: ({ text }: { text: string }) => <pre>{text}</pre> }));

const project: Project = { id: 'p', name: 'Work', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'WRK', githubRepo: null, kind: 'repo' };
const conn = (name: string, kind: string): Connection => ({ id: `c-${name}`, name, kind, url: '', config: {}, hasSecret: true, createdAt: 0, updatedAt: 0, lastError: null, lastErrorAt: null });
const config = { filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null, effective: { runs: [], tests: [] } };

describe('Jira and Buildkite', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('a Jira connection asks for the site, email and API token', async () => {
    const created = { ...conn('work', 'jira'), hasSecret: false };
    const calls = stubApi({ 'POST /api/connections': created, 'PUT /api/connections/c-work/secret': created });
    renderWithQuery(<ConnectionDialog kind="jira" onClose={() => {}} />);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Create and manage API tokens/)).toBeTruthy();
    await userEvent.type(within(dialog).getByLabelText('Name'), 'work');
    await userEvent.type(within(dialog).getByLabelText('Address'), 'https://acme.atlassian.net');
    await userEvent.type(within(dialog).getByLabelText('API token'), 'tok');
    const save = within(dialog).getByRole('button', { name: 'Save' });
    expect(save.hasAttribute('disabled') || save.getAttribute('aria-disabled') === 'true').toBe(true);
    await userEvent.type(within(dialog).getByLabelText('Email'), 'me@acme.com');
    await userEvent.click(save);
    await waitFor(() => expect(calls.some((c) => c.method === 'PUT')).toBe(true));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ name: 'work', kind: 'jira', url: 'https://acme.atlassian.net', config: { email: 'me@acme.com' } });
  });

  it('links the project to Jira and Buildkite in its settings, each field saved as it changes', async () => {
    let current = { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, integrations: {} as Record<string, unknown> };
    const calls = stubApi({
      'GET /api/projects/p/settings': () => current,
      'GET /api/projects/p/run-config': config,
      'GET /api/connections': [conn('work-jira', 'jira'), conn('bk', 'buildkite'), conn('alerts', 'discord_webhook')],
      'PUT /api/projects/p/settings': (c) => {
        const body = c.body as { integrations: Record<string, unknown> };
        current = { ...current, integrations: { ...current.integrations, ...body.integrations } };
        return current;
      },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    await userEvent.click(await screen.findByRole('button', { name: /Jira connection/ }));
    const options = within(screen.getByRole('listbox')).getAllByRole('option').map((o) => o.textContent);
    expect(options).toEqual(['work-jira']);
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'work-jira' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(await screen.findByRole('textbox', { name: 'Done in Jira' })).toHaveValue('Done');
    await userEvent.type(screen.getByRole('textbox', { name: 'Jira issues' }), 'project = ABC{Enter}');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    await userEvent.click(screen.getByRole('button', { name: /Buildkite connection/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'bk' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(3));
    await userEvent.type(await screen.findByRole('textbox', { name: 'Pipelines' }), 'app{enter}app-e2e');
    await userEvent.tab();
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(4));
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { agentCredentials: null, runOverrides: null, integrations: { jira: { connection: 'work-jira', scope: '', done: 'Done' } } },
      { agentCredentials: null, runOverrides: null, integrations: { jira: { connection: 'work-jira', scope: 'project = ABC', done: 'Done' } } },
      { agentCredentials: null, runOverrides: null, integrations: { buildkite: { connection: 'bk', pipelines: [] } } },
      { agentCredentials: null, runOverrides: null, integrations: { buildkite: { connection: 'bk', pipelines: ['app', 'app-e2e'] } } },
    ]);
  });

  it('shows a linked integration\'s settings and unlinks it', async () => {
    const linked = { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null,
      integrations: { buildkite: { connection: 'bk', pipelines: ['app', 'e2e'] } } };
    const calls = stubApi({
      'GET /api/projects/p/settings': linked,
      'GET /api/projects/p/run-config': config,
      'GET /api/connections': [conn('bk', 'buildkite')],
      'PUT /api/projects/p/settings': { ...linked, integrations: {} },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByRole('textbox', { name: 'Pipelines' })).toHaveValue('app\ne2e');
    expect(screen.queryByRole('button', { name: 'Unlink Jira' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Unlink Buildkite' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toMatchObject({ integrations: { buildkite: null } }));
  });

  it('imports an issue by key and opens its ticket, or says why not', async () => {
    const ticket = { id: 't9', projectId: 'p' };
    let first = true;
    const calls = stubApi({
      'POST /api/projects/p/jira/import': () => {
        if (first) { first = false; return new Reply(404, { code: 'jira_issue_not_found', message: 'Jira has no issue ABC-404' }); }
        return { ticket, existed: false };
      },
    });
    const open = vi.fn();
    const onClose = vi.fn();
    renderWithQuery(<OpenTicketContext.Provider value={open}><ImportJiraDialog projectId="p" onClose={onClose} /></OpenTicketContext.Provider>);
    const box = await screen.findByRole('textbox', { name: 'Issue key' });
    await userEvent.type(box, 'ABC-404{Enter}');
    expect(await screen.findByText('Jira has no issue ABC-404')).toBeTruthy();
    await userEvent.clear(box);
    await userEvent.type(box, ' abc-9 ');
    await userEvent.click(screen.getByRole('button', { name: 'Import' }));
    await waitFor(() => expect(open).toHaveBeenCalledWith(ticket));
    expect(onClose).toHaveBeenCalled();
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ key: 'ABC-404' }, { key: 'abc-9' }]);
  });

  it("shows a ticket's Jira key, as a link where it can, and a branch's own CI", () => {
    const links = [{ system: 'jira', key: 'ABC-1', url: 'https://acme.atlassian.net/browse/ABC-1' }];
    const { unmount } = renderWithQuery(<ExternalKeys links={links} linked />);
    expect(screen.getByRole('link', { name: /ABC-1/ }).getAttribute('href')).toBe(links[0].url);
    unmount();
    renderWithQuery(<><ExternalKeys links={links} /><CiBadge ci={{ state: 'failed', url: 'u', runs: [{ provider: 'buildkite', id: 'app/3', attempt: 1, name: 'app', state: 'failed', url: 'u', failedJobs: ['test'] }] }} /></>);
    expect(screen.queryByRole('link')).toBeNull();
    expect(screen.getByText('ABC-1')).toBeTruthy();
    expect(screen.getByText('CI failed').closest('[title]')?.getAttribute('title')).toBe('Failed: test');
  });

  it("shows a ticket's GitHub issue by its number, naming the repository", () => {
    const links = [{ system: 'github', key: 'o/r#12', url: 'https://github.com/o/r/issues/12' }];
    renderWithQuery(<ExternalKeys links={links} linked />);
    const link = screen.getByRole('link', { name: /#12/ });
    expect(link.getAttribute('href')).toBe(links[0].url);
    expect(link.closest('[title]')?.getAttribute('title')).toBe('GitHub issue o/r#12');
  });
});
