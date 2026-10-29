import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { GalleryTemplate } from '../api/gen/GalleryTemplate';
import { renderWithQuery, stubApi } from '../test-utils';
import { BLANK, CATEGORIES, gallery, matches, triggerLabel } from './gallery';
import { NewWorkflowDialog } from './NewWorkflowDialog';
import { DOC, detail } from './testData';

afterEach(() => vi.unstubAllGlobals());

function tpl(over: Partial<GalleryTemplate> & { id: string }): GalleryTemplate {
  return {
    name: over.id, description: '', builtin: true, category: 'other', params: [], doc: DOC, createdAt: 0, updatedAt: 0,
    plugin: null, builtinWorkflow: false, added: false, triggers: ['Run now'], steps: [], ...over,
  };
}

const LIST: GalleryTemplate[] = [
  tpl({ id: BLANK, name: 'Blank', doc: { ...DOC, name: 'New workflow' } }),
  tpl({ id: 'builtin:ci', name: 'Tell the agent when CI fails', category: 'ciPr', description: 'Sends the failed jobs.', triggers: ['CI run failed'], steps: ['Send to the agent'] }),
  tpl({ id: 'builtin:triage', name: 'Triage new GitHub issues', category: 'issues', triggers: ['GitHub issue opened'], steps: ['Triage'],
    plugin: { id: 'github', label: 'GitHub', on: true },
    params: [{ id: 'repo', title: 'Repository', description: '', type: 'string', choices: [], required: true }] }),
  tpl({ id: 'builtin:jira-done', name: 'Done moves the Jira issue', category: 'tickets', builtinWorkflow: true,
    plugin: { id: 'jira', label: 'Jira', on: false }, triggers: ['Ticket moved'], steps: ['Move Jira issue'] }),
  tpl({ id: 'builtin:pr-notices', name: 'PR notices', category: 'ciPr', builtinWorkflow: true, added: true,
    triggers: ['CI run failed', 'Conflict', 'Merged'], steps: Array.from({ length: 10 }, (_, i) => `Step ${i + 1}`) }),
  tpl({ id: 'builtin:pr-feedback', name: 'PR comments to the agent', category: 'ciPr', builtinWorkflow: true, added: false }),
  tpl({ id: 'u1', name: 'Mine', builtin: false, doc: { ...DOC, name: 'From mine' },
    params: [{ id: 'word', title: 'Word', description: '', type: 'string', choices: [], required: true },
      { id: 'loud', title: 'Loud', description: '', type: 'boolean', choices: [], default: true, required: false }] }),
];

describe('gallery model', () => {
  it('searches name, description and plugin', () => {
    const ci = LIST[1];
    expect([matches(ci, 'tell'), matches(ci, 'FAILED JOBS'), matches(ci, 'jira')]).toEqual([true, true, false]);
    expect(matches(LIST[3], 'jira')).toBe(true);
    expect(matches(ci, '  ')).toBe(true);
  });

  it('groups by category under All, Start from scratch first and built-ins apart; empty categories are hidden', () => {
    const g = gallery(LIST, '', 'all');
    expect(g.blank?.id).toBe(BLANK);
    expect(g.groups.map((x) => [x.id, x.items.map((t) => t.id)])).toEqual([
      ['ciPr', ['builtin:ci']], ['issues', ['builtin:triage']], ['other', ['u1']],
    ]);
    expect(g.builtins.map((t) => t.id)).toEqual(['builtin:jira-done', 'builtin:pr-notices', 'builtin:pr-feedback']);
    expect(g.counts.all).toBe(4);
    expect(g.counts.categories.map((c) => [c.id, c.count])).toEqual([['ciPr', 1], ['issues', 1], ['other', 1]]);
    expect(g.counts.builtin).toBe(3);
    expect(CATEGORIES.map((c) => c.id)).toEqual(['ciPr', 'tickets', 'issues', 'notifications', 'releases', 'other']);
  });

  it('filters by a category, by built-in, and by the search', () => {
    expect(gallery(LIST, '', 'ciPr')).toMatchObject({ blank: null, builtins: [], groups: [{ id: 'ciPr' }] });
    const b = gallery(LIST, '', 'builtin');
    expect([b.groups.length, b.builtins.length]).toEqual([0, 3]);
    const q = gallery(LIST, 'pr', 'all');
    expect(q.blank).toBeNull();
    expect(q.counts.categories.map((c) => [c.id, c.count])).toEqual([['ciPr', 0], ['issues', 0], ['other', 0]]);
    expect(q.builtins.map((t) => t.id)).toEqual(['builtin:pr-notices', 'builtin:pr-feedback']);
    expect(gallery(LIST, 'nothing like it', 'all').empty).toBe(true);
  });

  it('names several triggers by the first', () => {
    expect(triggerLabel(LIST[4])).toBe('CI run failed +2');
    expect(triggerLabel(tpl({ id: 'x', triggers: [] }))).toBe('No trigger');
  });
});

function open(templateId?: string) {
  const calls = stubApi({
    'GET /api/workflow-templates': LIST,
    'POST /api/workflow-templates/u1/instantiate': detail({ id: 'w9' }),
    'POST /api/workflow-templates/builtin%3Ablank/instantiate': detail({ id: 'w8' }),
  });
  const onCreated = vi.fn();
  const onClose = vi.fn();
  renderWithQuery(<NewWorkflowDialog projects={[{ id: 'p1', name: 'Proj' } as never]} templateId={templateId} onClose={onClose} onCreated={onCreated} />);
  return { calls, onCreated, onClose };
}

describe('new workflow gallery', () => {
  it('starts on Start from scratch with a blank name, and creates it', async () => {
    const { calls, onCreated } = open();
    const preview = await screen.findByRole('region', { name: 'Preview: Blank' });
    expect(within(preview).getByRole('textbox', { name: 'Name' })).toHaveValue('');
    await userEvent.click(within(preview).getByRole('button', { name: 'Create workflow' }));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith('w8'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ params: {}, name: null, scope: 'host', projectId: null });
  });

  it("previews a picked template's trigger and steps, then creates it with its params, name and project", async () => {
    const { calls, onCreated } = open();
    await userEvent.click(await screen.findByRole('button', { name: /^Mine/ }));
    const preview = screen.getByRole('region', { name: 'Preview: Mine' });
    expect(within(preview).getByRole('textbox', { name: 'Name' })).toHaveValue('From mine');
    await userEvent.type(within(preview).getByRole('textbox', { name: /Word/ }), 'hi');
    await userEvent.click(within(preview).getByRole('button', { name: /Project/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Proj' }));
    await userEvent.click(within(preview).getByRole('button', { name: 'Create workflow' }));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith('w9'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ params: { word: 'hi', loud: true }, name: 'From mine', scope: 'project', projectId: 'p1' });
  });

  it('filters by category and search, and offers Start from scratch when nothing matches', async () => {
    open();
    await screen.findByRole('button', { name: /^Mine/ });
    const nav = screen.getByRole('navigation', { name: 'Categories' });
    expect(within(nav).queryByRole('button', { name: /Tickets/ })).toBeNull();
    await userEvent.click(within(nav).getByRole('button', { name: /CI & pull requests/ }));
    expect(screen.getByRole('button', { name: /Tell the agent when CI fails/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Triage new GitHub issues/ })).toBeNull();
    await userEvent.click(within(nav).getByRole('button', { name: /^All/ }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Search templates' }), 'zzz');
    expect(screen.getByText(/No templates match/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('link', { name: 'Start from scratch' }));
    expect(screen.getByRole('textbox', { name: 'Search templates' })).toHaveValue('');
    expect(screen.getByRole('region', { name: 'Preview: Blank' })).toBeInTheDocument();
  });

  it('keeps built-ins in a collapsed group marked Added or Not added; an added one warns of a second copy', async () => {
    open();
    const toggle = await screen.findByRole('button', { name: /Built-in · 3/ });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('button', { name: /PR notices/ })).toBeNull();
    await userEvent.click(toggle);
    const notices = screen.getByRole('button', { name: /PR notices/ });
    expect(within(notices).getByText('Added')).toBeInTheDocument();
    expect(within(screen.getByRole('button', { name: /PR comments to the agent/ })).getByText('Not added')).toBeInTheDocument();
    await userEvent.click(notices);
    const preview = screen.getByRole('region', { name: 'Preview: PR notices' });
    expect(within(preview).getByText(/This adds a second copy/)).toBeInTheDocument();
    expect(within(preview).getByText(/and 2 more steps/)).toBeInTheDocument();
    expect(within(preview).getByRole('button', { name: 'Create workflow' })).toBeEnabled();
  });

  it("fades a template whose plugin is off, says so, links to its page, and can't create it", async () => {
    const { onClose } = open('builtin:jira-done');
    const card = await screen.findByRole('button', { name: /Done moves the Jira issue/ });
    expect(within(card).getByText('Jira is off')).toBeInTheDocument();
    const preview = screen.getByRole('region', { name: 'Preview: Done moves the Jira issue' });
    expect(within(preview).getByText(/Needs the Jira plugin/)).toBeInTheDocument();
    expect(within(preview).getByRole('button', { name: 'Create workflow' })).toBeDisabled();
    await userEvent.click(within(preview).getByRole('link', { name: 'Turn on Jira' }));
    expect(onClose).toHaveBeenCalled();
    await waitFor(() => expect(window.location.pathname).toBe('/settings/plugins/jira'));
  });
});

describe('new workflow gallery on a phone', () => {
  it('shows categories as chips and the picked template as a sheet', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query.includes('max-width'), media: query, addEventListener: () => {}, removeEventListener: () => {},
    }));
    open();
    await screen.findByRole('button', { name: /^Mine/ });
    const chips = screen.getByRole('group', { name: 'Categories' });
    expect(within(chips).getByRole('button', { name: 'Built-in' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: /Preview/ })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: /^Mine/ }));
    const sheet = screen.getByRole('dialog', { name: 'Mine' });
    expect(within(sheet).getByRole('button', { name: 'Create workflow' })).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole('button', { name: 'Back to templates' }));
    expect(screen.queryByRole('dialog', { name: 'Mine' })).toBeNull();
  });
});
