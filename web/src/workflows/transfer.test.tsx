import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ImportCheck } from '../api/gen/ImportCheck';
import type { ImportRequest } from '../api/gen/ImportRequest';
import type { WorkflowExport } from '../api/gen/WorkflowExport';
import { Reply, renderWithQuery, stubApi, type Call, stubClipboard } from '../test-utils';
import { ExportDialog } from './ExportDialog';
import { ImportDialog } from './ImportDialog';
import { CATALOG, DOC, detail } from './testData';
import { WorkflowPage } from './WorkflowPage';

afterEach(() => vi.unstubAllGlobals());

const EXPORT: WorkflowExport = {
  format: 'pdt-workflow', formatVersion: 1, pdtVersion: '1.0.0', exportedAt: 1,
  workflows: [{ name: DOC.name, description: '', scope: 'host', doc: DOC }],
  requires: { connections: [{ name: 'alerts', kind: 'discord_webhook', tools: [] }], personas: [], projects: [] },
};

const clipboard = () => {
  const writeText = vi.fn(async (_: string) => {});
  stubClipboard({ writeText });
  return writeText;
};

describe('export', () => {
  it('copies a workflow export from its page', async () => {
    stubApi({ 'GET /api/workflows/w1': detail(), 'POST /api/workflows/check': { issues: [], ports: {} }, 'GET /api/workflows/w1/export': EXPORT });
    const writeText = clipboard();
    renderWithQuery(<WorkflowPage id="w1" catalog={CATALOG} projects={[]} onOpen={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'More workflow actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Copy export' }));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    const text = writeText.mock.calls[0][0];
    expect(JSON.parse(text)).toEqual(EXPORT);
    expect(text).toContain('\n  "format": "pdt-workflow"');
    expect(await screen.findByText('Copied')).toBeInTheDocument();
  });

  it('shows the text of the picked workflows, to copy or download', async () => {
    const calls = stubApi({ 'POST /api/workflows/export': { ...EXPORT, workflows: [EXPORT.workflows[0], { ...EXPORT.workflows[0], name: 'Two' }] } });
    const writeText = clipboard();
    const w = (id: string, name: string) => ({ ...detail(), id, name });
    renderWithQuery(<ExportDialog workflows={[w('a', 'One'), w('b', 'Two'), w('c', 'Three')]} onClose={() => {}} />);
    await userEvent.click(screen.getByRole('checkbox', { name: 'Three' }));
    await userEvent.click(screen.getByRole('button', { name: 'Export 2' }));
    const box = await screen.findByRole('textbox', { name: 'Export' });
    expect(calls.find((c) => c.path === '/api/workflows/export')?.body).toEqual({ ids: ['a', 'b'] });
    expect((box as HTMLTextAreaElement).value).toContain('"name": "Two"');
    expect(screen.getByRole('button', { name: 'Download' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith((box as HTMLTextAreaElement).value));
  });
});

const CHECK: ImportCheck = {
  pdtVersion: '1.0.0',
  workflows: [{ name: DOC.name, description: '', scope: 'host', project: null, nodes: 2, existing: [], issues: [] }],
  connections: [{ name: 'alerts', kind: 'discord_webhook', tools: [], status: 'missing' }],
  personas: [],
  projects: [],
  warnings: [],
};

describe('import', () => {
  function importDialog(result: unknown = { workflows: [{ ...detail(), id: 'new' }], warnings: [] }) {
    const calls = stubApi({
      'POST /api/workflows/import': (c: Call) => {
        const b = c.body as ImportRequest;
        if (b.text.startsWith('{nope')) return new Reply(400, { code: 'invalid_export', message: 'this is not valid JSON: key must be a string' });
        return b.check ? CHECK : result;
      },
      'GET /api/connections': [{ id: 'c1', name: 'team-alerts', kind: 'discord_webhook', url: '', config: {}, hasSecret: true, createdAt: 1, updatedAt: 1, lastError: null, lastErrorAt: null }],
      'GET /api/personas': [],
    });
    const onImported = vi.fn();
    renderWithQuery(<ImportDialog projects={[]} onClose={() => {}} onImported={onImported} />);
    return { calls, onImported };
  }

  it('imports pasted text once every missing name is mapped', async () => {
    const { calls, onImported } = importDialog();
    const box = screen.getByRole('textbox', { name: 'Export to import' });
    fireEvent.change(box, { target: { value: JSON.stringify(EXPORT, null, 2) } });
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));
    const needs = await screen.findByRole('region', { name: 'What it needs' });
    expect(within(needs).getByText('Missing')).toBeInTheDocument();
    const go = () => screen.getByRole('button', { name: 'Import 1 workflow' });
    expect(go()).toBeDisabled();
    await userEvent.click(within(needs).getByRole('button', { name: /Map alerts/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'team-alerts' }));
    await waitFor(() => expect(go()).toBeEnabled());
    await userEvent.click(go());
    await waitFor(() => expect(onImported).toHaveBeenCalledWith('new'));
    const sent = calls.filter((c) => c.path === '/api/workflows/import').at(-1)!.body as ImportRequest;
    expect(sent.check).toBe(false);
    expect(sent.mapping.connections).toEqual({ alerts: 'team-alerts' });
    expect(sent.targets).toEqual([{ scope: 'host', copy: false, skip: false }]);
  });

  it('can leave a name unmapped, and shows what is left to do', async () => {
    importDialog({ workflows: [{ ...detail(), id: 'new' }], warnings: ['the discord_webhook connection alerts is left unmapped'] });
    fireEvent.change(screen.getByRole('textbox', { name: 'Export to import' }), { target: { value: JSON.stringify(EXPORT) } });
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));
    await userEvent.click(await screen.findByRole('button', { name: /Map alerts/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Leave unmapped' }));
    await userEvent.click(screen.getByRole('button', { name: 'Import 1 workflow' }));
    expect(await screen.findByText(/alerts is left unmapped/)).toBeInTheDocument();
  });

  it('fills the box from a chosen file', async () => {
    importDialog();
    const file = new File([JSON.stringify(EXPORT)], 'w.pdt-workflow.json', { type: 'application/json' });
    const input = document.querySelector('input[type=file]') as HTMLInputElement;
    await userEvent.upload(input, file);
    await waitFor(() => expect((screen.getByRole('textbox', { name: 'Export to import' }) as HTMLTextAreaElement).value).toContain('pdt-workflow'));
  });

  it('shows why text cannot be imported', async () => {
    importDialog();
    fireEvent.change(screen.getByRole('textbox', { name: 'Export to import' }), { target: { value: '{nope' } });
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));
    expect(await screen.findByText(/not valid JSON/)).toBeInTheDocument();
  });
});
