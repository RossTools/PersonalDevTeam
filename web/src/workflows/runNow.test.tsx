import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { TopBarActionsContext, type TopBarActions } from '../shell/PhoneTopBar';
import { Reply, renderWithQuery, stubApi, type Call } from '../test-utils';
import { confirmText, initialValues, problem, runForm, runInput, visibleFields } from './runInputs';
import { CATALOG, detail } from './testData';
import { WorkflowPage } from './WorkflowPage';

afterEach(() => vi.unstubAllGlobals());

const doc = (config: Record<string, unknown> = {}): WorkflowDoc => ({
  name: 'Cut a release',
  description: '',
  nodes: [
    { id: 'start', kind: 'manual', version: 1, config, position: { x: 0, y: 0 } },
    { id: 'end', kind: 'end', version: 1, config: {}, position: { x: 300, y: 0 } },
  ],
  edges: [{ from: 'start', port: 'out', to: 'end' }],
  settings: { maxSteps: 200, maxRuntimeMs: 604800000 },
} as WorkflowDoc);

/** Cut a release's Run now trigger, as its template makes it with a Nightly. */
const RELEASE = {
  inputs: [
    { id: 'type', label: 'Type', type: 'choice', required: true, default: 'prerelease',
      choices: [{ value: 'prerelease', label: 'Nightly' }, { value: 'stable', label: 'Stable' }] },
    { id: 'version', label: 'Exact version', type: 'text', required: false, showIf: { field: 'type', equals: 'stable' },
      help: 'X.Y.Z. Leave it empty to bump the version instead.' },
    { id: 'bump', label: 'Bump', type: 'choice', required: false, default: 'patch', showIf: { field: 'version', equals: '' },
      choices: [{ value: 'patch', label: 'Patch' }, { value: 'minor', label: 'Minor' }, { value: 'major', label: 'Major' }] },
  ],
  confirm: { text: 'This publishes a stable release ({{trigger.bump}}{{trigger.version}}). Continue?', showIf: { field: 'type', equals: 'stable' } },
};

function page(config: Record<string, unknown>) {
  const calls = stubApi({
    'GET /api/workflows/w1': detail({ name: 'Cut a release', doc: doc(config) }),
    'POST /api/workflows/check': { issues: [], ports: {} },
    'POST /api/workflows/w1/run': { runId: null },
  });
  renderWithQuery(<WorkflowPage id="w1" catalog={CATALOG} projects={[]} onOpen={() => {}} />);
  return calls;
}

const runs = (calls: Call[]) => calls.filter((c) => c.method === 'POST' && c.path.endsWith('/run'));

describe('Run now', () => {
  it('runs at once when the trigger declares no inputs', async () => {
    const calls = page({});
    await userEvent.click(await screen.findByRole('button', { name: 'Run now' }));
    await waitFor(() => expect(runs(calls).map((c) => c.body)).toEqual([{ input: {} }]));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('asks for the declared inputs, Nightly first, and a stable release only after confirming', async () => {
    const calls = page(RELEASE);
    await userEvent.click(await screen.findByRole('button', { name: 'Run now' }));
    const dlg = await screen.findByRole('dialog', { name: 'Run Cut a release' });
    expect(within(dlg).getByRole('radio', { name: 'Nightly' })).toBeChecked();
    expect(within(dlg).queryByRole('radiogroup', { name: 'Bump' })).not.toBeInTheDocument();

    // Stable shows the version and the bump; an exact version hides the bump.
    await userEvent.click(within(dlg).getByRole('radio', { name: 'Stable' }));
    expect(within(dlg).getByRole('radio', { name: 'Patch' })).toBeChecked();
    await userEvent.type(within(dlg).getByRole('textbox', { name: 'Exact version' }), '2.0.0');
    expect(within(dlg).queryByRole('radiogroup', { name: 'Bump' })).not.toBeInTheDocument();
    await userEvent.clear(within(dlg).getByRole('textbox', { name: 'Exact version' }));
    await userEvent.click(within(dlg).getByRole('radio', { name: 'Minor' }));

    await userEvent.click(within(dlg).getByRole('button', { name: 'Run now' }));
    expect(within(dlg).getByText('This publishes a stable release (minor). Continue?')).toBeInTheDocument();
    expect(runs(calls)).toEqual([]);
    await userEvent.click(within(dlg).getByRole('button', { name: 'Continue' }));
    await waitFor(() => expect(runs(calls).map((c) => c.body)).toEqual([{ input: { type: 'stable', bump: 'minor' } }]));
  });

  it('starts a nightly without asking, and shows a refusal in the form', async () => {
    const calls = stubApi({
      'GET /api/workflows/w1': detail({ name: 'Cut a release', doc: doc(RELEASE) }),
      'POST /api/workflows/check': { issues: [], ports: {} },
      'POST /api/workflows/w1/run': new Reply(409, { code: 'cannot_run', message: 'Cut a release can\'t run with that input' }),
    });
    renderWithQuery(<WorkflowPage id="w1" catalog={CATALOG} projects={[]} onOpen={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Run now' }));
    const dlg = await screen.findByRole('dialog', { name: 'Run Cut a release' });
    await userEvent.click(within(dlg).getByRole('button', { name: 'Run now' }));
    await waitFor(() => expect(runs(calls).map((c) => c.body)).toEqual([{ input: { type: 'prerelease' } }]));
    expect(await within(dlg).findByText("Cut a release can't run with that input")).toBeInTheDocument();
  });

  it('is in ⋯ on a phone, with the same form', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query.includes('max-width'), media: query, addEventListener: () => {}, removeEventListener: () => {},
    }));
    stubApi({ 'GET /api/workflows/w1': detail({ name: 'Cut a release', doc: doc(RELEASE) }), 'POST /api/workflows/check': { issues: [], ports: {} } });
    let actions: { entries: TopBarActions } | null = null;
    renderWithQuery(
      <TopBarActionsContext.Provider value={(a) => { actions = a; }}>
        <WorkflowPage id="w1" catalog={CATALOG} projects={[]} onOpen={() => {}} />
      </TopBarActionsContext.Provider>,
    );
    await waitFor(() => expect(actions).not.toBeNull());
    const entries = actions!.entries();
    expect(entries.map((e) => ('label' in e ? e.label : null))).toEqual(['Run now', 'Enable…']);
    act(() => (entries[0] as { onSelect: () => void }).onSelect());
    expect(await screen.findByRole('dialog', { name: 'Run Cut a release' })).toBeInTheDocument();
    // The outline says what Run now asks for.
    expect(screen.getByText('Asks for Type, Exact version, Bump')).toBeInTheDocument();
  });
});

describe('run inputs', () => {
  const form = runForm(doc(RELEASE))!;

  it('reads the declared inputs and fills in the defaults', () => {
    expect(runForm(doc({}))).toBeNull();
    expect(runForm(doc({ confirm: { text: 'Sure?' } }))?.fields).toEqual([]);
    const values = initialValues(form.fields);
    expect(values).toEqual({ type: 'prerelease', version: '', bump: 'patch' });
    expect(visibleFields(form.fields, values).map((f) => f.id)).toEqual(['type']);
    expect(runInput(form.fields, values)).toEqual({ type: 'prerelease' });
    expect(confirmText(form.confirm, runInput(form.fields, values))).toBeNull();
  });

  it('shows a field while the one it names is shown and has the value', () => {
    const stable = { type: 'stable', version: '', bump: 'major' };
    expect(visibleFields(form.fields, stable).map((f) => f.id)).toEqual(['type', 'version', 'bump']);
    expect(runInput(form.fields, { ...stable, version: '1.2.3' })).toEqual({ type: 'stable', version: '1.2.3' });
    expect(confirmText(form.confirm, runInput(form.fields, { ...stable, version: '1.2.3' })))
      .toBe('This publishes a stable release (1.2.3). Continue?');
  });

  it('needs the required fields and numbers that are numbers', () => {
    const fields = [
      { id: 'why', label: 'Why', type: 'text', required: true },
      { id: 'n', label: '', type: 'number', required: false },
    ] as Parameters<typeof problem>[0];
    expect(problem(fields, { why: '', n: '' })).toBe('Why is required');
    expect(problem(fields, { why: 'x', n: 'lots' })).toBe('n must be a number');
    expect(problem(fields, { why: 'x', n: '3' })).toBeNull();
    expect(runInput(fields, { why: 'x', n: '3' })).toEqual({ why: 'x', n: 3 });
  });
});
