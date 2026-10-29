import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { TopBarActionsContext, type TopBarActions } from '../shell/PhoneTopBar';
import { Reply, renderWithQuery, stubApi, type Call } from '../test-utils';
import { DryRunDialog } from './DryRunDialog';
import { RunsView } from './RunsView';
import { classify } from './SchemaForm';
import { CATALOG, DOC, detail } from './testData';
import { TextView } from './TextView';
import { VersionsView } from './VersionsView';
import { WorkflowPage } from './WorkflowPage';
import { groupWorkflows } from './WorkflowsList';
import { fromText, toText } from './textFormat';

afterEach(() => vi.unstubAllGlobals());

/** Below the md breakpoint: every max-width media query matches (the phone layout). */
const stubPhone = () =>
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('max-width'),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));

const check = { issues: [], ports: { ci: [{ id: 'out', title: 'Then' }], tell: [{ id: 'sent', title: 'Told' }, { id: 'untold', title: 'No session to tell' }, { id: 'skipped', title: 'Already sent' }] } };

function page(over = {}) {
  const calls = stubApi({
    'GET /api/workflows/w1': detail(over),
    'POST /api/workflows/check': check,
    'PUT /api/workflows/w1': (c: Call) => detail({ version: 2, doc: (c.body as { doc: WorkflowDoc }).doc }),
    'PATCH /api/workflows/w1': (c: Call) => ({ ...detail(), ...(c.body as object) }),
  });
  renderWithQuery(<WorkflowPage id="w1" catalog={CATALOG} projects={[]} onOpen={() => {}} />);
  return calls;
}

describe('workflow page', () => {
  it('draws the graph, edits the draft and saves it as a version', async () => {
    const calls = page();
    expect(await screen.findByText('CI failed')).toBeInTheDocument();
    expect(screen.getByText('Tell the agent')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Unsaved changes' })).not.toBeInTheDocument();
    const name = within(screen.getByRole('region', { name: 'Workflow details' })).getByRole('textbox', { name: 'Name' });
    await userEvent.clear(name);
    await userEvent.type(name, 'Renamed');
    const bar = screen.getByRole('region', { name: 'Unsaved changes' });
    await userEvent.type(within(bar).getByRole('textbox', { name: 'Note for this version' }), 'new name');
    await waitFor(() => expect(within(bar).getByRole('button', { name: 'Save' })).toBeEnabled());
    await userEvent.click(within(bar).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toMatchObject({ note: 'new name', propose: false, doc: { name: 'Renamed' } }));
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Unsaved changes' })).not.toBeInTheDocument());
  });

  it('removes a step from the outline, and undo brings it back', async () => {
    page();
    await userEvent.click(await screen.findByRole('radio', { name: 'Outline' }));
    const steps = screen.getByRole('list', { name: 'Steps' });
    expect(within(steps).getByText('Tell the agent')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Actions for Tell the agent' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Remove' }));
    await waitFor(() => expect(within(steps).queryByText('Tell the agent')).not.toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
    expect(within(steps).getByText('Tell the agent')).toBeInTheDocument();
  });

  it('asks before turning a workflow on, saying what it runs on and can do', async () => {
    const calls = page();
    await userEvent.click(await screen.findByRole('switch', { name: 'Enabled' }));
    const dlg = await screen.findByRole('dialog', { name: /Turn on CI to agent/ });
    expect(within(dlg).getByText('CI failed')).toBeInTheDocument();
    expect(within(dlg).getByText('Tell the agent')).toBeInTheDocument();
    await userEvent.click(within(dlg).getByRole('button', { name: 'Turn on' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ enabled: true }));
  });

  it("is read-only when it lives in the repo, and shows an agent's proposal", async () => {
    page({ source: 'repo', repoFile: '.pdt/workflows/ci.json', proposed: 3 });
    expect(await screen.findByText(/lives in the repo \(\.pdt\/workflows\/ci\.json\)/)).toBeInTheDocument();
    expect(screen.getByText('An agent proposed version 3.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Add node' })).not.toBeInTheDocument();
  });
});

describe('workflow page on a phone', () => {
  it('opens on the outline, shows nothing that edits, and enables from ⋯', async () => {
    stubPhone();
    const calls = stubApi({
      'GET /api/workflows/w1': detail({ enabled: false }),
      'POST /api/workflows/check': check,
      'PATCH /api/workflows/w1': (c: Call) => ({ ...detail(), ...(c.body as object) }),
    });
    let actions: { entries: TopBarActions } | null = null;
    renderWithQuery(
      <TopBarActionsContext.Provider value={(a) => { actions = a; }}>
        <WorkflowPage id="w1" catalog={CATALOG} projects={[]} onOpen={() => {}} />
      </TopBarActionsContext.Provider>,
    );
    const steps = await screen.findByRole('list', { name: 'Steps' });
    expect(within(steps).getByText('Tell the agent')).toBeInTheDocument();
    // Outline first on a phone.
    expect(screen.getByRole('radiogroup', { name: 'View' })).toHaveTextContent(/^OutlineRunsGraphTextVersions$/);
    expect(screen.getByRole('radio', { name: 'Outline' })).toBeChecked();
    for (const name of ['Dry run', 'Run now', 'More workflow actions', 'Add trigger', 'Actions for Tell the agent']) {
      expect(screen.queryByRole('button', { name })).not.toBeInTheDocument();
    }
    expect(screen.queryByRole('switch', { name: 'Enabled' })).not.toBeInTheDocument();
    // Read-only on a phone is not "in the repo": this one lives in PDT.
    expect(screen.queryByText('In the repo')).not.toBeInTheDocument();
    // A step opens read-only.
    await userEvent.click(within(steps).getByText('Tell the agent'));
    const sheet = await screen.findByRole('dialog');
    for (const box of within(sheet).queryAllByRole('textbox')) expect(box).toHaveAttribute('readonly');
    await userEvent.keyboard('{Escape}');
    // ⋯ has Enable…, which asks first (without a dry run on the phone).
    await waitFor(() => expect(actions).not.toBeNull());
    const entries = actions!.entries();
    expect(entries.map((e) => ('label' in e ? e.label : null))).toEqual(['Enable…']);
    act(() => (entries[0] as { onSelect: () => void }).onSelect());
    const ask = await screen.findByRole('dialog', { name: 'Turn on CI to agent?' });
    expect(within(ask).queryByRole('button', { name: 'Dry run first' })).not.toBeInTheDocument();
    await userEvent.click(within(ask).getByRole('button', { name: 'Turn on' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ enabled: true }));
  });
});

describe('text view', () => {
  it('applies a document that reads and shows why one does not', async () => {
    const onChange = vi.fn();
    renderWithQuery(<TextView doc={DOC} onChange={onChange} />);
    const editor = await screen.findByRole('textbox', { name: 'Workflow as JSON' });
    expect(editor.textContent).toContain('"name": "CI to agent"');
    await userEvent.click(screen.getByRole('radio', { name: 'TOML' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Workflow as TOML' }).textContent).toContain('name = "CI to agent"'));
  });

  it('round-trips JSON and TOML, leaving out nulls', () => {
    const doc = { ...DOC, nodes: [{ ...DOC.nodes[0], timeoutMs: null }, DOC.nodes[1]] } as WorkflowDoc;
    const back = fromText(toText(doc, 'toml'), 'toml');
    expect('doc' in back && back.doc.nodes[1].config).toEqual(DOC.nodes[1].config);
    expect('doc' in back && 'timeoutMs' in back.doc.nodes[0]).toBe(false);
    expect(fromText('{"name": 1}', 'json')).toEqual({ error: 'name must be text' });
    expect('error' in fromText('{', 'json')).toBe(true);
  });
});

describe('runs view', () => {
  const run = (status: string, steps: object[]) => ({
    id: 'r1', workflowId: 'w1', workflowName: 'CI to agent', version: 1, status, runKey: 'k', triggerEventId: null, triggerNode: 'ci',
    trigger: { type: 'ci.run_failed', ticketKey: 'PDT-1' }, projectId: null, ticketId: null, attemptId: null, parentRunId: null,
    parentStepId: null, error: status === 'failed' ? 'tell failed' : null, output: null, wakeAt: null, startedAt: 1, resumedAt: null, endedAt: null, steps,
    resumeFrom: null as string | null,
  });
  const step = (id: string, nodeId: string, status: string, extra = {}) => ({
    id, runId: 'r1', seq: id === 's1' ? 1 : 2, nodeId, kind: nodeId === 'ci' ? 'ci.run_failed' : 'gate', status, input: {}, output: null,
    port: status === 'succeeded' ? 'out' : null, attempts: 1, error: null, wakeAt: null, wait: null, startedAt: 1, endedAt: null, ...extra,
  });

  it('lists runs, retries a failed one and answers a waiting gate', async () => {
    const failed = run('failed', [step('s1', 'ci', 'succeeded'), step('s2', 'tell', 'failed', { error: 'no session' })]);
    const waiting = { ...run('waiting', [step('s1', 'ci', 'succeeded'), step('s2', 'tell', 'waiting', { wait: { answer: { question: 'Go on?', choices: ['yes', 'no'] } } })]), id: 'r2' };
    const calls = stubApi({
      'GET /api/workflow-runs': [failed, waiting],
      'GET /api/workflow-runs/r1': failed,
      'GET /api/workflow-runs/r2': waiting,
      'GET /api/workflows/w1/versions/1': { workflowId: 'w1', version: 1, author: 'user', status: 'saved', note: '', createdAt: 1, doc: DOC },
      'POST /api/workflows/check': check,
      'POST /api/workflow-runs/r1/retry': { ...failed, status: 'running' },
      'POST /api/workflow-runs/r2/steps/s2/answer': { ...waiting, status: 'running' },
    });
    let runId: string | null = null;
    const { rerender } = renderWithQuery(<RunsView workflowId="w1" catalog={CATALOG} runId={runId} onRunChange={(id) => { runId = id; }} />);
    const detail = await screen.findByRole('region', { name: 'Run' });
    expect(within(detail).getByText('tell failed')).toBeInTheDocument();
    expect(await within(detail).findByText('no session')).toBeInTheDocument();
    await userEvent.click(within(detail).getByRole('button', { name: 'Retry from the failed step' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path.endsWith('/retry'))).toBe(true));
    await userEvent.click(within(screen.getByRole('navigation', { name: 'Runs' })).getAllByRole('button')[1]);
    rerender(<RunsView workflowId="w1" catalog={CATALOG} runId={runId} onRunChange={() => {}} />);
    expect(await screen.findByText('Go on?')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'yes' }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith('/answer'))?.body).toEqual({ choice: 'yes' }));
  });

  it('on a phone: the list is its own screen, and a run is its steps, each opening to its input and output', async () => {
    const failed = run('failed', [step('s1', 'ci', 'succeeded', { input: { ticket: 'PDT-1' } }), step('s2', 'tell', 'failed', { error: 'no session' })]);
    stubApi({
      'GET /api/workflow-runs': [failed],
      'GET /api/workflow-runs/r1': failed,
      'GET /api/workflows/w1/versions/1': { workflowId: 'w1', version: 1, author: 'user', status: 'saved', note: '', createdAt: 1, doc: DOC },
      'POST /api/workflows/check': check,
    });
    const onRunChange = vi.fn();
    const { rerender } = renderWithQuery(<RunsView workflowId="w1" catalog={CATALOG} runId={null} onRunChange={onRunChange} viewOnly />);
    const list = await screen.findByRole('navigation', { name: 'Runs' });
    await userEvent.click(await within(list).findByRole('button', { name: /ci\.run_failed/ }));
    expect(onRunChange).toHaveBeenCalledWith('r1');
    expect(screen.queryByRole('region', { name: 'Run' })).not.toBeInTheDocument();
    rerender(<RunsView workflowId="w1" catalog={CATALOG} runId="r1" onRunChange={onRunChange} viewOnly />);
    const detail = await screen.findByRole('region', { name: 'Run' });
    expect(screen.queryByRole('navigation', { name: 'Runs' })).not.toBeInTheDocument();
    expect(within(detail).queryByRole('button', { name: 'Retry from the failed step' })).not.toBeInTheDocument();
    expect(within(detail).queryByRole('application')).not.toBeInTheDocument();
    expect(await within(detail).findByText('no session')).toBeInTheDocument();
    const steps = within(detail).getByRole('list', { name: 'Steps' });
    const first = within(steps).getAllByRole('button')[0];
    expect(first).toHaveAttribute('aria-expanded', 'false');
    expect(within(steps).queryByText(/"ticket": "PDT-1"/)).not.toBeInTheDocument();
    await userEvent.click(first);
    expect(first).toHaveAttribute('aria-expanded', 'true');
    expect(within(steps).getByText(/"ticket": "PDT-1"/)).toBeInTheDocument();
  });

  it('resumes a run from the agent step it timed out at', async () => {
    const ended = { ...run('succeeded', [step('s1', 'ci', 'succeeded'), step('s2', 'tell', 'succeeded', { port: 'timeout' })]), resumeFrom: 's2' };
    const calls = stubApi({
      'GET /api/workflow-runs': [ended],
      'GET /api/workflow-runs/r1': ended,
      'GET /api/workflows/w1/versions/1': { workflowId: 'w1', version: 1, author: 'user', status: 'saved', note: '', createdAt: 1, doc: DOC },
      'POST /api/workflows/check': check,
      'POST /api/workflow-runs/r1/resume': { ...ended, status: 'running', resumeFrom: null },
    });
    renderWithQuery(<RunsView workflowId="w1" catalog={CATALOG} runId={null} onRunChange={() => {}} />);
    const detail = await screen.findByRole('region', { name: 'Run' });
    await userEvent.click(within(detail).getByRole('button', { name: 'Resume from tell' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path.endsWith('/resume'))).toBe(true));
    await waitFor(() => expect(within(detail).queryByRole('button', { name: 'Resume from tell' })).not.toBeInTheDocument());
  });
});

describe('versions', () => {
  it("shows a proposal's diff and accepts it", async () => {
    const calls = stubApi({
      'GET /api/workflows/w1/versions': [
        { workflowId: 'w1', version: 2, author: 'agent', status: 'proposed', note: 'add a step', createdAt: 2 },
        { workflowId: 'w1', version: 1, author: 'user', status: 'saved', note: '', createdAt: 1 },
      ],
      'GET /api/workflows/w1/diff': { nodesAdded: ['note'], nodesRemoved: [], nodesChanged: [], edgesAdded: [], edgesRemoved: [], metaChanged: false, unified: '--- v1\n+++ v2\n@@ -1,1 +1,2 @@\n {\n+  "x": 1\n' },
      'POST /api/workflows/w1/versions/2/accept': detail({ version: 2 }),
    });
    const onChanged = vi.fn();
    renderWithQuery(<VersionsView w={detail({ proposed: 2 })} dirty={false} onChanged={onChanged} />);
    expect(await screen.findByText('added note')).toBeInTheDocument();
    expect(screen.getByText(/An agent proposed this change/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Accept' }));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(calls.some((c) => c.path === '/api/workflows/w1/versions/2/accept')).toBe(true);
  });
});

describe('dry run', () => {
  it('runs the draft against a written event and shows what it would do', async () => {
    const calls = stubApi({
      'GET /api/events': [],
      'POST /api/workflows/check': check,
      'POST /api/workflows/dry-run': {
        issues: [],
        run: { triggers: [{ node: 'ci', matched: true, reason: null }], status: 'succeeded', steps: [
          { nodeId: 'ci', kind: 'ci.run_failed', input: {}, port: 'out', output: {}, error: null },
          { nodeId: 'tell', kind: 'session.send', input: {}, port: 'sent', output: { would: 'tell the agent: CI failed' }, error: null },
        ] },
      },
    });
    renderWithQuery(<DryRunDialog open onOpenChange={() => {}} doc={DOC} workflowId="w1" catalog={CATALOG} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Write one' }));
    const payload = screen.getByRole('textbox', { name: 'Payload' });
    expect((payload as HTMLTextAreaElement).value).toContain('"attemptId"');
    fireEvent.change(payload, { target: { value: '{"ticketId": "t1"}' } });
    await userEvent.click(within(screen.getByRole('dialog')).getAllByRole('button', { name: 'Dry run' }).at(-1)!);
    expect(await screen.findByText('Would tell the agent: CI failed')).toBeInTheDocument();
    expect(screen.getByText('Would end: Succeeded')).toBeInTheDocument();
    expect(calls.find((c) => c.path === '/api/workflows/dry-run')?.body).toMatchObject({ doc: DOC, workflowId: 'w1', event: { type: 'ci.run_failed', payload: { ticketId: 't1' } }, eventId: null });
  });

  it('reports a dry run the server refused', async () => {
    stubApi({ 'GET /api/events': [], 'POST /api/workflows/dry-run': new Reply(400, { code: 'invalid_input', message: 'give event or eventId' }) });
    renderWithQuery(<DryRunDialog open onOpenChange={() => {}} doc={DOC} catalog={CATALOG} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Write one' }));
    await userEvent.click(within(screen.getByRole('dialog')).getAllByRole('button', { name: 'Dry run' }).at(-1)!);
    expect(await screen.findByText('give event or eventId')).toBeInTheDocument();
  });
});

describe('schema form', () => {
  const kind = (id: string) => CATALOG.find((k) => k.id === id)!.configSchema as Record<string, unknown>;
  const prop = (id: string, key: string) => classify((kind(id).properties as Record<string, Record<string, unknown>>)[key], kind(id), key);

  it('reads the kinds of field the catalog uses', () => {
    expect(prop('gate', 'choices').field.kind).toBe('strings');
    expect(prop('gate', 'ahead').field.kind).toBe('objects');
    expect(prop('gate', 'ask').field).toMatchObject({ kind: 'enum', options: [{ value: 'wait' }, { value: 'skip' }] });
    expect(prop('session.send', 'text').field).toEqual({ kind: 'string', multiline: true });
    expect(prop('session.send', 'attempt')).toMatchObject({ field: { kind: 'string' }, nullable: true });
    expect(prop('ticket.finish', 'deleteBranch')).toMatchObject({ field: { kind: 'boolean' }, tpl: true });
    expect(prop('wait', 'ms')).toMatchObject({ field: { kind: 'number', integer: true }, tpl: true, nullable: true });
    expect(prop('wait', 'event').field.kind).toBe('object');
  });
});

describe('workflows list', () => {
  it('groups by where workflows apply: this host, personal, then projects by name', () => {
    const w = (id: string, scope: string, projectId: string | null = null) => ({ ...detail({ id, scope: scope as never, projectId }) });
    const groups = groupWorkflows([w('a', 'project', 'p2'), w('b', 'personal'), w('c', 'host'), w('d', 'project', 'p1')],
      [{ id: 'p1', name: 'Alpha' } as never, { id: 'p2', name: 'Beta' } as never]);
    expect(groups.map((g) => [g.label, g.items.map((i) => i.id)])).toEqual([['This host', ['c']], ['Personal', ['b']], ['Alpha', ['d']], ['Beta', ['a']]]);
  });
});
