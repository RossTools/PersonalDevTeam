import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { AppShell } from './AppShell';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

afterEach(() => vi.unstubAllGlobals());

const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null };

function stubAlpha() {
  stubApi({
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: [shell],
  });
}

describe('AppShell', () => {
  it('renders the five rail views and marks the active one', async () => {
    stubApi({ 'GET /api/projects': [] });
    renderWithQuery(<AppShell />);
    for (const name of ['Inbox', 'Board', 'Sessions', 'Personas', 'Files']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Board' }));
    expect(screen.getByRole('button', { name: 'Board' })).toHaveAttribute('aria-current', 'page');
  });

  it('toggles the bottom panel with Ctrl+` and with the on-screen button', async () => {
    stubApi({ 'GET /api/projects': [] });
    renderWithQuery(<AppShell />);
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    const toggle = screen.getByRole('button', { name: 'Toggle terminal panel' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toHaveTextContent('Select a project to open a terminal.');
  });

  it('shows the selected project name, not its id', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    const main = screen.getByRole('main');
    expect(main).toHaveTextContent('Project Alpha');
    expect(main).not.toHaveTextContent(project.id);
    expect(screen.getByRole('button', { name: 'Alpha' })).toHaveAttribute('aria-current', 'true');
  });

  it('opens a selected session in the work area and shells as panel tabs', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    // The session row's name includes its StatusDot label ("running" + "Shell", no space in jsdom), unlike "New Shell in Repository".
    await userEvent.click(await screen.findByRole('button', { name: /^running\s*Shell$/ }));
    expect(screen.getByRole('main')).toContainElement(screen.getByTestId('terminal-view'));
    await userEvent.click(screen.getByRole('button', { name: 'Toggle terminal panel' }));
    expect(await screen.findByRole('tab', { name: /Shell/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('button', { name: 'Maximize terminal panel' }));
    expect(screen.getByRole('button', { name: 'Restore terminal panel' })).toBeInTheDocument();
    expect(screen.getAllByTestId('terminal-view')).toHaveLength(1);
  });
});
