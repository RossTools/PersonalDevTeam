// One workflow (Phase 7d spec §2): the header (Enabled, Dry run, Run now, ⋯), the draft bar, and the Graph, Outline,
// Text, Runs and Versions views. The three builder views edit one draft; Save makes a version. On a phone it is
// view-only (PDT-120): Outline first, nothing that edits, and Enable/Disable under the top bar's ⋯; an agent's proposal
// can still be accepted or discarded there (PDT-320).
import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Redo2, Undo2 } from 'lucide-react';
import {
  Alert, Badge, Banner, Button, Dialog, Field, IconButton, Input, MenuButton, SegmentedControl, Sheet, Skeleton, StatusDot, Textarea,
  ViewHeader, toast, useConfirm, type MenuEntry,
} from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';
import { runAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { DryRunDialog } from './DryRunDialog';
import { EnableDialog, EnableSwitch, useEnable } from './EnableSwitch';
import { ExportDialog } from './ExportDialog';
import { GraphView } from './GraphView';
import { hasErrors, type Catalog } from './model';
import { NodePanel } from './NodePanel';
import { OutlineView } from './OutlineView';
import { RunNowDialog } from './RunNowDialog';
import { runForm, type RunValues } from './runInputs';
import { RunsView } from './RunsView';
import { scopeLabel } from './scope';
import { TextView } from './TextView';
import { copyExport } from './transfer';
import { useCheck, useDraft } from './useDraft';
import { useProposal } from './useProposal';
import { VersionsView } from './VersionsView';

export type WorkflowTab = 'graph' | 'outline' | 'text' | 'runs' | 'versions';

/** Issue codes for what the workflow names that is not on this host (an imported workflow's, PDT-155). */
const SETUP_CODES = ['missing_connection', 'connection_kind', 'hook_secret', 'missing_persona', 'missing_project'];

/** The workflow's own settings, shown beside the builder when no node is selected. */
function DetailsPanel({ doc, onChange, readOnly }: { doc: WorkflowDoc; onChange: (d: WorkflowDoc) => void; readOnly: boolean }) {
  return (
    <section aria-label="Workflow details" className="flex flex-col gap-3 p-3">
      <h3 className="text-sm font-medium">Workflow</h3>
      <Field label="Name">
        <Input value={doc.name} readOnly={readOnly} onChange={(e) => onChange({ ...doc, name: e.target.value })} />
      </Field>
      <Field label="Description">
        <Textarea rows={3} value={doc.description} readOnly={readOnly} onChange={(e) => onChange({ ...doc, description: e.target.value })} />
      </Field>
      <Field label="Steps per run" hint="A run that takes more fails (a guard against loops).">
        <Input type="number" min={1} value={String(doc.settings.maxSteps)} readOnly={readOnly}
          onChange={(e) => onChange({ ...doc, settings: { ...doc.settings, maxSteps: Math.max(1, Math.trunc(Number(e.target.value)) || 1) } })} />
      </Field>
      <Field label="Longest run (hours)" hint="Waits included.">
        <Input type="number" min={1} value={String(Math.round(doc.settings.maxRuntimeMs / 3_600_000))} readOnly={readOnly}
          onChange={(e) => onChange({ ...doc, settings: { ...doc.settings, maxRuntimeMs: Math.max(1, Number(e.target.value) || 1) * 3_600_000 } })} />
      </Field>
      <p className="text-meta text-muted-foreground">Select a node to configure it.</p>
    </section>
  );
}

function SaveAsTemplateDialog({ w, doc, onClose, onSaved }: { w: WorkflowDetail; doc: WorkflowDoc; onClose: () => void; onSaved: (id: string) => void }) {
  const [name, setName] = useState(w.name);
  const [description, setDescription] = useState(w.description);
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.createWorkflowTemplate({ name, description, params: [], doc }),
    onSuccess: (t) => onSaved(t.id),
  });
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }} title="Save as template" dismissable={false}
      description="A copy of this workflow you can start new ones from. Add parameters on the template's page."
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!name.trim()} pending={save.isPending} onClick={() => save.mutate()}>Save template</Button></>}>
      <div className="flex flex-col gap-3">
        <Field label="Name"><Input value={name} onChange={(e) => setName(e.target.value)} /></Field>
        <Field label="Description"><Textarea rows={3} value={description} onChange={(e) => setDescription(e.target.value)} /></Field>
        {save.error && <Alert tone="error">{save.error.message}</Alert>}
      </div>
    </Dialog>
  );
}

export interface WorkflowPageProps {
  id: string;
  catalog: Catalog;
  projects: Project[];
  /** Another workflow or a template to show (Duplicate, Save as template); null after Delete. */
  onOpen: (target: { kind: 'workflow' | 'template'; id: string } | null) => void;
  /** The tab and run shown, when something else holds them (the app's route, PDT-111); without it the page keeps its own. */
  view?: WorkflowPageView;
  /** The project in view: Run now's default when the workflow asks for one. */
  projectInView?: string | null;
}

export interface WorkflowPageView {
  /** Unset: the default (Outline on a phone, Graph otherwise). */
  tab?: WorkflowTab;
  runId: string | null;
  onChange: (tab: WorkflowTab, runId: string | null) => void;
}

export function WorkflowPage({ id, catalog, projects, onOpen, view, projectInView }: WorkflowPageProps) {
  // Polled, so a proposal or a save made elsewhere shows (the draft is never replaced while it has changes).
  const w = useQuery({ queryKey: ['workflow', id], queryFn: () => api.getWorkflow(id), refetchInterval: 5000 });
  if (w.error) return <Alert tone="error" className="m-3">The workflow could not be loaded: {w.error.message}</Alert>;
  if (!w.data) return <Skeleton lines={8} className="p-4" />;
  return <Loaded w={w.data} catalog={catalog} projects={projects} onOpen={onOpen} view={view} projectInView={projectInView} />;
}

function Loaded({ w, catalog, projects, onOpen, view, projectInView }: { w: WorkflowDetail } & Omit<WorkflowPageProps, 'id'>) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const phone = useMediaQuery(PHONE_QUERY);
  const [ownTab, setOwnTab] = useState<WorkflowTab | undefined>(undefined);
  const [ownRunId, setOwnRunId] = useState<string | null>(null);
  const tab: WorkflowTab = (view ? view.tab : ownTab) ?? (phone ? 'outline' : 'graph');
  const runId = view ? view.runId : ownRunId;
  const show = (t: WorkflowTab, r: string | null) => {
    if (view) view.onChange(t, r);
    else { setOwnTab(t); setOwnRunId(r); }
  };
  const setTab = (t: WorkflowTab) => show(t, t === 'runs' ? runId : null);
  const setRunId = (r: string | null) => show('runs', r);
  const [selected, setSelected] = useState<string | null>(null);
  const [dryRun, setDryRun] = useState(false);
  const [note, setNote] = useState('');
  const [templating, setTemplating] = useState(false);
  const [exporting, setExporting] = useState(false);
  // Run now's form is open (the workflow declares inputs or a confirmation).
  const [asking, setAsking] = useState(false);
  // Workflows are view-only on a phone.
  const readOnly = w.source === 'repo' || phone;
  const enable = useEnable(w);
  const blocked = hasErrors({ issues: w.issues, ports: {} }) ? 'Fix the errors first' : null;
  const hasManual = w.doc.nodes.some((n) => n.kind === 'manual');
  // Run now runs the saved version, so its form is the saved trigger's; it asks for the project when the steps need one.
  const form = runForm(w.doc) ?? (w.needsProject ? { fields: [], confirm: null } : null);
  useTopBarActions(phone ? () => [
    ...(hasManual ? [{ label: 'Run now', onSelect: () => startRun() }] : []),
    w.enabled
      ? { label: 'Disable', onSelect: () => enable.toggle.mutate(false, { onError: (e) => toast.error(e.message) }) }
      : { label: 'Enable…', disabled: !!blocked, onSelect: () => enable.setAsking(true) },
  ] : null);
  const draft = useDraft(w.doc, `${w.id}:${w.version}`);
  const check = useCheck(draft.doc).data;
  const proposal = useProposal(w, draft.dirty, () => draft.discard());
  const errors = hasErrors(check);
  // A node removed in another view (or by undo) closes its panel.
  useEffect(() => {
    if (selected && !draft.doc.nodes.some((n) => n.id === selected)) setSelected(null);
  }, [draft.doc, selected]);

  const saved = (d: WorkflowDetail) => {
    qc.setQueryData(['workflow', d.id], d);
    void qc.invalidateQueries({ queryKey: ['workflows'] });
    void qc.invalidateQueries({ queryKey: ['workflow-versions', d.id] });
  };
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveWorkflow(w.id, { doc: draft.doc, note: note.trim(), propose: false }),
    onSuccess: (d) => { setNote(''); saved(d); toast.success(`Saved version ${d.version}`); },
  });
  const runNow = useMutation({
    mutationFn: (input: RunValues) => api.runWorkflow(w.id, input),
    onSuccess: (r) => {
      setAsking(false);
      void qc.invalidateQueries({ queryKey: ['workflow-runs'] });
      if (r.runId) setRunId(r.runId); else toast.info('A run with that key already ran');
    },
    // The form shows its own error.
    onError: (e) => { if (!form) toast.error(e.message); },
  });
  // Without declared inputs (or a confirmation) Run now starts at once.
  const startRun = () => {
    runNow.reset();
    if (form) setAsking(true);
    else runNow.mutate({});
  };
  const menu = (): MenuEntry[] => [
    { label: 'Copy export', onSelect: () => void copyExport([w.id]) },
    { label: 'Export…', onSelect: () => setExporting(true) },
    { separator: true },
    { label: 'Save as template…', onSelect: () => setTemplating(true) },
    {
      label: 'Duplicate', onSelect: () => void runAction(async () => {
        const copy = await api.createWorkflow({
          doc: { ...draft.doc, name: `${draft.doc.name} copy` }, scope: w.scope, projectId: w.projectId, enabled: false, note: `copied from ${w.name}`,
        });
        void qc.invalidateQueries({ queryKey: ['workflows'] });
        onOpen({ kind: 'workflow', id: copy.id });
      }),
    },
    ...(w.builtin || readOnly ? [] : [
      { separator: true } as const,
      {
        label: 'Delete…', danger: true, onSelect: () => void runAction(async () => {
          const { ok } = await confirm({ title: `Delete ${w.name}?`, body: 'Its versions and runs go with it.', action: 'Delete workflow', danger: true });
          if (!ok) return;
          await api.deleteWorkflow(w.id);
          void qc.invalidateQueries({ queryKey: ['workflows'] });
          onOpen(null);
        }),
      },
    ]),
  ];

  const builder = tab === 'graph' || tab === 'outline';
  const node = selected ? draft.doc.nodes.find((n) => n.id === selected) : undefined;
  const edit = readOnly ? undefined : draft.set;
  const panel = node ? (
    <NodePanel doc={draft.doc} node={node} catalog={catalog} check={check} onChange={draft.set} onSelect={setSelected} onClose={() => setSelected(null)} readOnly={readOnly} />
  ) : (
    <DetailsPanel doc={draft.doc} onChange={draft.set} readOnly={readOnly} />
  );
  const issueCount = (check?.issues ?? []).filter((i) => i.severity === 'error').length;
  const setup = w.issues.filter((i) => SETUP_CODES.includes(i.code));

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title={draft.doc.name || 'Untitled'}
        leading={<StatusDot status={w.enabled ? 'running' : 'idle'} label={w.enabled ? 'On' : 'Off'} />}
        meta={
          <>
            <span>{scopeLabel(w, projects)}</span>
            <span>v{w.version}</span>
            {w.builtin && <Badge tone="neutral">Built in</Badge>}
            {w.source === 'repo' && <Badge tone="neutral">In the repo</Badge>}
            {issueCount > 0 && <Badge tone="failed">{issueCount === 1 ? '1 error' : `${issueCount} errors`}</Badge>}
          </>
        }
        actions={phone ? undefined : (
          <>
            <EnableSwitch w={w} catalog={catalog} onDryRun={() => setDryRun(true)} blocked={blocked} />
            <Button size="sm" variant="secondary" onClick={() => setDryRun(true)}>Dry run</Button>
            {hasManual && <Button size="sm" variant="secondary" disabled={draft.dirty} pending={runNow.isPending && !asking} onClick={startRun}
              title={draft.dirty ? 'Save first: Run now runs the saved version' : undefined}>Run now</Button>}
            <MenuButton label="More workflow actions" entries={menu} />
          </>
        )}
      />
      {w.source === 'repo' && <Banner tone="neutral">This workflow lives in the repo ({w.repoFile}): edit the file to change it.</Banner>}
      {setup.length > 0 && (
        <Banner tone="waiting">
          <span className="flex flex-col">
            <span>Not set up on this host yet:</span>
            {setup.map((i) => <span key={`${i.node}:${i.code}`}>{i.message}</span>)}
          </span>
        </Banner>
      )}
      {w.proposed !== null && (
        <Banner tone="waiting" actions={
          <>
            {tab !== 'versions' && <Button size="sm" variant="ghost" onClick={() => setTab('versions')}>Review</Button>}
            {w.source !== 'repo' && (
              <>
                <Button size="sm" variant="secondary" pending={proposal.discarding} onClick={() => proposal.discard(w.proposed!)}>Discard</Button>
                <Button size="sm" variant="primary" pending={proposal.accepting} onClick={() => void proposal.accept(w.proposed!)}>Accept</Button>
              </>
            )}
          </>
        }>
          An agent proposed version {w.proposed}.{proposal.error ? ` ${proposal.error.message}` : ''}
        </Banner>
      )}
      {draft.dirty && (
        <div role="region" aria-label="Unsaved changes" className="flex flex-wrap items-center gap-2 border-b border-border bg-accent px-3 py-1.5 text-sm">
          <span className="font-medium">Unsaved changes</span>
          {draft.savedElsewhere && <span className="text-waiting">A newer version was saved elsewhere.</span>}
          <IconButton label="Undo" size="sm" disabled={!draft.canUndo} onClick={draft.undo}><Undo2 aria-hidden /></IconButton>
          <IconButton label="Redo" size="sm" disabled={!draft.canRedo} onClick={draft.redo}><Redo2 aria-hidden /></IconButton>
          <Input size="sm" aria-label="Note for this version" placeholder="What changed (optional)" value={note} onChange={(e) => setNote(e.target.value)} className="w-56 max-w-full" />
          <span className="ml-auto flex gap-1">
            <Button size="sm" variant="secondary" onClick={draft.discard}>Discard</Button>
            <Button size="sm" variant="primary" disabled={errors} title={errors ? 'Fix the errors first' : undefined} pending={save.isPending} onClick={() => save.mutate()}>Save</Button>
          </span>
          {save.error && <span className="basis-full text-destructive">{save.error.message}</span>}
        </div>
      )}
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 overflow-x-auto">
        <SegmentedControl<WorkflowTab> aria-label="View" size="sm" value={tab} onChange={setTab} options={phone ? [
          { value: 'outline', label: 'Outline' }, { value: 'runs', label: 'Runs' }, { value: 'graph', label: 'Graph' },
          { value: 'text', label: 'Text' }, { value: 'versions', label: 'Versions' },
        ] : [
          { value: 'graph', label: 'Graph' }, { value: 'outline', label: 'Outline' }, { value: 'text', label: 'Text' },
          { value: 'runs', label: 'Runs' }, { value: 'versions', label: 'Versions' },
        ]} />
        {builder && phone && <Button size="sm" variant="ghost" className="ml-auto" onClick={() => setSelected(selected ?? '')}>Details</Button>}
      </div>
      <div className="flex min-h-0 flex-1">
        {tab === 'graph' && <GraphView doc={draft.doc} onChange={edit} catalog={catalog} check={check} selected={selected || null} onSelect={setSelected} readOnly={readOnly} />}
        {tab === 'outline' && <OutlineView doc={draft.doc} onChange={edit} catalog={catalog} check={check} selected={selected || null} onSelect={setSelected} readOnly={readOnly} />}
        {tab === 'text' && <TextView doc={draft.doc} onChange={edit} readOnly={readOnly} />}
        {tab === 'runs' && <RunsView workflowId={w.id} catalog={catalog} runId={runId} onRunChange={setRunId} viewOnly={phone} />}
        {tab === 'versions' && <VersionsView w={w} dirty={draft.dirty} readOnly={readOnly} onChanged={() => draft.discard()} />}
        {builder && !phone && <aside aria-label="Panel" className="w-80 shrink-0 overflow-auto border-l border-border">{panel}</aside>}
      </div>
      {builder && phone && (
        <Sheet side="bottom" title={node ? node.id : 'Workflow'} open={selected !== null} onOpenChange={(o) => { if (!o) setSelected(null); }} className="max-h-[85vh]">
          <div className="-mx-4 overflow-auto">{panel}</div>
        </Sheet>
      )}
      {phone && <EnableDialog w={w} catalog={catalog} enable={enable} />}
      {dryRun && <DryRunDialog open onOpenChange={setDryRun} doc={draft.doc} workflowId={w.id} catalog={catalog} />}
      {asking && form && (
        <RunNowDialog name={w.name} form={form} projects={w.needsProject ? projects : undefined} project={projectInView}
          pending={runNow.isPending} error={runNow.error?.message ?? null}
          onRun={(input) => runNow.mutate(input)} onClose={() => setAsking(false)} />
      )}
      {exporting && <ExportDialog ids={[w.id]} onClose={() => setExporting(false)} />}
      {templating && <SaveAsTemplateDialog w={w} doc={draft.doc} onClose={() => setTemplating(false)} onSaved={(tid) => { setTemplating(false); void qc.invalidateQueries({ queryKey: ['workflow-templates'] }); onOpen({ kind: 'template', id: tid }); }} />}
    </div>
  );
}
