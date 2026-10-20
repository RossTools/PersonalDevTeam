import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Workflow } from '../api/gen/Workflow';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { WorkflowsSection } from './WorkflowsSection';

afterEach(() => vi.unstubAllGlobals());

const wf = (id: string, over: Partial<Workflow> = {}): Workflow => ({
  id, name: `Flow ${id}`, description: `Does ${id}`, scope: 'host', projectId: null, enabled: true, builtin: null,
  source: 'host', repoFile: null, version: 1, createdAt: 1, updatedAt: 1, ...over,
});

describe('workflows settings', () => {
  it('lists the workflows with their last run and turns one off', async () => {
    const calls = stubApi({
      'GET /api/workflows': [wf('a', { name: 'PR notices', builtin: 'pr-notices' }), wf('b', { enabled: false, source: 'repo' })],
      'GET /api/workflow-runs': [{ id: 'r2', workflowId: 'a', status: 'failed', startedAt: 2 }, { id: 'r1', workflowId: 'a', status: 'succeeded', startedAt: 1 }],
      'PATCH /api/workflows/a': (c: Call) => wf('a', { ...(c.body as object) }),
    });
    renderWithQuery(<WorkflowsSection />);
    const toggle = await screen.findByRole('switch', { name: 'Workflow PR notices' });
    expect(toggle).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Workflow Flow b' })).not.toBeChecked();
    expect(screen.getByText('Built in')).toBeInTheDocument();
    expect(screen.getByText('In the repo')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.queryByText('Succeeded')).not.toBeInTheDocument();
    await userEvent.click(toggle);
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ enabled: false }));
  });
});
