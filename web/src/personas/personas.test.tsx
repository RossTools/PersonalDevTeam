import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Persona } from '../api/gen/Persona';
import type { PersonaVersion } from '../api/gen/PersonaVersion';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { PersonaPage } from './PersonaPage';
import { PersonasList } from './PersonasList';

afterEach(() => vi.unstubAllGlobals());

const persona = (id: string, over: Partial<Persona> = {}): Persona => ({
  id, name: `Persona ${id}`, role: 'coder', description: '', builtin: null, archived: false, version: 1, createdAt: 1, updatedAt: 1,
  agent: 'claude', model: null, instructions: 'Build it.', permissionMode: null, tools: [], ...over,
});
const version = (v: number, over: Partial<PersonaVersion> = {}): PersonaVersion => ({
  personaId: 'a', version: v, author: 'user', note: '', createdAt: 1, agent: 'claude', model: null, instructions: '', permissionMode: null, tools: [], ...over,
});

describe('PersonasList', () => {
  it('lists personas with role, agent and model, archived last, and opens New persona', async () => {
    stubApi({
      'GET /api/personas': [
        persona('z', { name: 'Archivist', archived: true }),
        persona('b', { name: 'Reviewer', role: 'reviewer', agent: 'codex', model: 'gpt-5' }),
        persona('a', { name: 'Coder' }),
      ],
    });
    const onSelect = vi.fn();
    renderWithQuery(<PersonasList selectedId="b" onSelect={onSelect} />);
    const list = await screen.findByRole('grid', { name: 'Personas' });
    const rows = within(list).getAllByRole('row');
    expect(rows.map((r) => r.textContent)).toEqual(['Codercoder · Claude', 'Reviewerreviewer · Codex · gpt-5', 'ArchivistArchivedcoder · Claude']);
    await userEvent.click(rows[0]);
    expect(onSelect).toHaveBeenCalledWith('a');
    await userEvent.click(screen.getByRole('button', { name: 'New persona' }));
    expect(onSelect).toHaveBeenCalledWith('new');
  });
});

describe('PersonaPage', () => {
  it('asks to choose a persona when none is chosen', () => {
    stubApi({ 'GET /api/personas': [] });
    renderWithQuery(<PersonaPage personaId={null} onSelect={() => {}} />);
    expect(screen.getByText('Choose a persona')).toBeInTheDocument();
  });

  it('creates a persona and selects it', async () => {
    const calls = stubApi({ 'GET /api/personas': [], 'POST /api/personas': persona('n') });
    const onSelect = vi.fn();
    renderWithQuery(<PersonaPage personaId="new" onSelect={onSelect} />);
    await userEvent.type(screen.getByRole('textbox', { name: 'Name' }), 'Writer');
    await userEvent.type(screen.getByRole('textbox', { name: 'Role' }), 'docs');
    await userEvent.type(screen.getByRole('textbox', { name: 'Model' }), 'opus');
    await userEvent.click(screen.getByRole('checkbox', { name: /workflow_get/ }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Instructions' }), 'Write docs.');
    await userEvent.click(screen.getByRole('button', { name: 'Create persona' }));
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('n'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      name: 'Writer', role: 'docs', description: '', agent: 'claude', model: 'opus', instructions: 'Write docs.', permissionMode: null,
      tools: ['get_ticket', 'submit_outcome', 'ask_user', 'add_ticket_comment', 'add_review_comment', 'workflow_get'],
    });
  });

  it('renames with PATCH and saves the setup as a new version with a note', async () => {
    const calls = stubApi({
      'GET /api/personas': [persona('a', { version: 2 })],
      'GET /api/personas/a/versions': [version(2), version(1)],
      'GET /api/personas/a/diff?from=1&to=2': { changed: [], unified: '' },
      'PATCH /api/personas/a': (c: Call) => persona('a', c.body as object),
      'PUT /api/personas/a': persona('a', { version: 3 }),
    });
    renderWithQuery(<PersonaPage personaId="a" onSelect={() => {}} />);
    const identity = await screen.findByRole('region', { name: 'Persona' });
    const name = within(identity).getByRole('textbox', { name: 'Name' });
    await userEvent.clear(name);
    await userEvent.type(name, 'Builder');
    await userEvent.click(within(identity).getByRole('button', { name: 'Save persona' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/personas/a', body: { name: 'Builder' } }));

    const setup = screen.getByRole('region', { name: 'Setup · version 2' });
    const save = within(setup).getByRole('button', { name: 'Save a new version' });
    expect(save).toBeDisabled();
    await userEvent.click(within(setup).getByRole('button', { name: /Permission mode/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Plan' }));
    await userEvent.type(within(setup).getByRole('textbox', { name: 'Version note' }), 'read only');
    await userEvent.click(save);
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({
      note: 'read only', agent: 'claude', model: null, instructions: 'Build it.', permissionMode: 'plan', tools: [],
    }));
  });

  it('previews the instructions as Markdown', async () => {
    stubApi({ 'GET /api/personas': [persona('a', { instructions: '**Bold** move' })], 'GET /api/personas/a/versions': [version(1)] });
    renderWithQuery(<PersonaPage personaId="a" onSelect={() => {}} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Preview' }));
    expect(screen.getByText('Bold').tagName).toBe('STRONG');
    expect(screen.queryByRole('textbox', { name: 'Instructions' })).toBeNull();
  });

  it('shows the versions newest first and the diff of the two picked', async () => {
    const calls = stubApi({
      'GET /api/personas': [persona('a', { version: 3 })],
      'GET /api/personas/a/versions': [version(3, { note: 'stricter', author: 'agent' }), version(2), version(1, { author: 'builtin' })],
      'GET /api/personas/a/diff?from=2&to=3': { changed: ['permissionMode'], unified: '--- v2\n+++ v3\n@@ -1 +1 @@\n-permission: (default)\n+permission: plan\n' },
      'GET /api/personas/a/diff?from=1&to=3': { changed: ['instructions'], unified: '--- v1\n+++ v3\n@@ -1 +1 @@\n-old\n+new\n' },
    });
    renderWithQuery(<PersonaPage personaId="a" onSelect={() => {}} />);
    const versions = await screen.findByRole('list', { name: 'Versions' });
    const rows = within(versions).getAllByRole('listitem');
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining('v3CurrentAn agentstricter'),
      expect.stringContaining('v2YouNo note'),
      expect.stringContaining('v1PDT (built in)'),
    ]);
    const diff = await screen.findByRole('region', { name: 'Changes from version 2 to 3' });
    expect(await within(diff).findByText('permissionMode')).toBeInTheDocument();
    expect(within(diff).getByText('permission: plan')).toBeInTheDocument();
    // Pick v1: the oldest pick (v2) drops.
    await userEvent.click(screen.getByRole('checkbox', { name: 'Compare version 1' }));
    const other = await screen.findByRole('region', { name: 'Changes from version 1 to 3' });
    expect(await within(other).findByText('instructions')).toBeInTheDocument();
    expect(calls.some((c) => c.path === '/api/personas/a/diff?from=1&to=3')).toBe(true);
  });

  it('archives a built-in persona and offers no Delete for it', async () => {
    const calls = stubApi({
      'GET /api/personas': [persona('a', { builtin: 'coder' })],
      'GET /api/personas/a/versions': [version(1, { author: 'builtin' })],
      'PATCH /api/personas/a': persona('a', { builtin: 'coder', archived: true }),
    });
    renderWithQuery(<PersonaPage personaId="a" onSelect={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Archive' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/personas/a', body: { archived: true } }));
    expect(screen.queryByRole('button', { name: 'Delete persona…' })).toBeNull();
  });

  it('deletes a persona after confirming, and toasts a refusal', async () => {
    let refuse = true;
    const calls = stubApi({
      'GET /api/personas': [persona('a'), persona('b', { name: 'Other' })],
      'GET /api/personas/a/versions': [version(1)],
      'DELETE /api/personas/a': () => (refuse ? ((refuse = false), new Reply(409, { code: 'persona_in_use', message: 'Persona a runs in Pipeline' })) : undefined),
    });
    const onSelect = vi.fn();
    renderWithQuery(<PersonaPage personaId="a" onSelect={onSelect} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Delete persona…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog', { name: 'Delete Persona a?' })).getByRole('button', { name: 'Delete persona' }));
    expect(await screen.findByText('Persona a runs in Pipeline')).toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Delete persona…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Delete persona' }));
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('b'));
    expect(calls.filter((c) => c.method === 'DELETE')).toHaveLength(2);
  });
});
