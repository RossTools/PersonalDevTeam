// Import… (PDT-155): paste an export (or a bare document from a workflow's Text view), or choose or drop its file into
// the same box; Check shows what it holds and needs here. Each missing connection, persona and project is mapped to one
// here or left unmapped (nothing imports until each is), each workflow gets where it applies and, on a name clash, a
// copy or a new version. Workflows import off; what is left to do shows after.
import { useRef, useState, type DragEvent } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Dialog, Field, Select, Textarea } from '@ui';
import { api } from '../api/client';
import type { ImportCheck } from '../api/gen/ImportCheck';
import type { ImportMapping } from '../api/gen/ImportMapping';
import type { ImportPreview } from '../api/gen/ImportPreview';
import type { ImportResult } from '../api/gen/ImportResult';
import type { ImportTarget } from '../api/gen/ImportTarget';
import type { Project } from '../api/gen/Project';
import { kindLabel } from '../connections/kinds';

/** The Select value for "leave it unmapped". */
const UNMAPPED = '\u0000unmapped';
type Group = keyof ImportMapping;
/** Per group, a needed name → a local one, or null for unmapped; a name not in it is unanswered. */
type Answers = Record<Group, Record<string, string | null>>;
const noAnswers = (): Answers => ({ connections: {}, personas: {}, projects: {} });

/** Where a workflow goes (empty: as exported) and what it does on a name clash (`copy`, or the id it versions). */
interface Choice { where: string; clash: string }

/** Where a workflow goes: the user's pick, else its scope, a project workflow in its project as mapped or found. */
function whereOf(c: Choice | undefined, w: ImportPreview, check: ImportCheck, answers: Answers): string {
  if (c?.where) return c.where;
  if (w.scope !== 'project') return w.scope;
  if (!w.project) return '';
  return answers.projects[w.project] ?? check.projects.find((p) => p.key === w.project)?.localId ?? '';
}

function NeedRow({ label, detail, status, options, value, onChange }: {
  label: string; detail?: string; status: string; options: { value: string; label: string }[];
  value: string | null | undefined; onChange: (v: string | null) => void;
}) {
  const found = status === 'found';
  return (
    <li className="flex flex-wrap items-center gap-2">
      <span className="min-w-0 flex-1 truncate text-sm">
        {label} {detail && <span className="text-muted-foreground">{detail}</span>}
      </span>
      <Badge tone={found ? 'running' : status === 'kind_mismatch' ? 'failed' : 'waiting'}>
        {found ? 'Found' : status === 'kind_mismatch' ? 'Other kind here' : 'Missing'}
      </Badge>
      {!found && (
        <Select size="sm" className="w-48" aria-label={`Map ${label}`} placeholder="Map to…"
          options={[...options, { value: UNMAPPED, label: 'Leave unmapped' }]}
          value={value === null ? UNMAPPED : value} onChange={(v) => onChange(v === UNMAPPED ? null : v)} />
      )}
    </li>
  );
}

function Report({ check, answers, setAnswers, choices, setChoices, projects }: {
  check: ImportCheck; answers: Answers; setAnswers: (a: Answers) => void;
  choices: Choice[]; setChoices: (c: Choice[]) => void; projects: Project[];
}) {
  const connections = useQuery({ queryKey: ['connections'], queryFn: api.listConnections });
  const personas = useQuery({ queryKey: ['personas'], queryFn: api.listPersonas });
  const answer = (g: Group, name: string, v: string | null) => setAnswers({ ...answers, [g]: { ...answers[g], [name]: v } });
  const choose = (i: number, c: Partial<Choice>) => setChoices(choices.map((x, n) => (n === i ? { ...x, ...c } : x)));
  const needs = check.connections.length + check.personas.length + check.projects.length;
  return (
    <div className="flex flex-col gap-3">
      <section aria-label="Workflows" className="flex flex-col gap-2">
        <h3 className="text-sm font-medium">{check.workflows.length === 1 ? '1 workflow' : `${check.workflows.length} workflows`}
          {check.pdtVersion && <span className="font-normal text-muted-foreground"> from PDT {check.pdtVersion}</span>}</h3>
        {check.workflows.map((w, i) => {
          const errors = w.issues.filter((x) => x.severity === 'error');
          return (
            <div key={i} className="flex flex-col gap-1.5 rounded-md border border-border p-2">
              <span className="text-sm font-medium">{w.name} <span className="font-normal text-muted-foreground">· {w.nodes} steps</span></span>
              <Field label="Applies to">
                <Select className="w-full" value={whereOf(choices[i], w, check, answers) || null} placeholder="Pick a project" onChange={(v) => choose(i, { where: v })}
                  options={[
                    { value: 'host', label: 'This host (every project)' },
                    ...projects.map((p) => ({ value: p.id, label: p.name })),
                    { value: 'personal', label: 'Personal (no project)' },
                  ]} />
              </Field>
              {w.existing.length > 0 && (
                <Field label={`A workflow is already called ${w.name}`}>
                  <Select className="w-full" value={choices[i]?.clash ?? 'copy'} onChange={(v) => choose(i, { clash: v })}
                    options={[
                      { value: 'copy', label: `Import as a copy (${w.name} (imported))` },
                      ...w.existing.filter((x) => x.source !== 'repo').map((x) => ({ value: x.id, label: `A new version of ${x.name} (${x.scope})` })),
                    ]} />
                </Field>
              )}
              {errors.length > 0 && <Alert tone="error">{errors.map((x) => x.message).join('; ')}</Alert>}
            </div>
          );
        })}
      </section>
      {needs > 0 && (
        <section aria-label="What it needs" className="flex flex-col gap-1.5">
          <h3 className="text-sm font-medium">What it needs here</h3>
          <ul className="flex flex-col gap-1.5">
            {check.connections.map((c) => (
              <NeedRow key={`c:${c.name}`} label={c.name} detail={`${kindLabel(c.kind)} connection${c.tools.length ? `: ${c.tools.join(', ')}` : ''}${c.localKind ? `; here it is ${kindLabel(c.localKind)}` : ''}`}
                status={c.status} value={answers.connections[c.name]} onChange={(v) => answer('connections', c.name, v)}
                options={(connections.data ?? []).filter((l) => c.kind === 'any' || l.kind === c.kind).map((l) => ({ value: l.name, label: l.name }))} />
            ))}
            {check.personas.map((p) => (
              <NeedRow key={`p:${p.name}`} label={p.name} detail="persona" status={p.status}
                value={answers.personas[p.name]} onChange={(v) => answer('personas', p.name, v)}
                options={(personas.data ?? []).filter((l) => !l.archived).map((l) => ({ value: l.name, label: l.name }))} />
            ))}
            {check.projects.map((p) => (
              <NeedRow key={`r:${p.key}`} label={p.key} detail={p.name ? `project ${p.name}` : 'project'} status={p.status}
                value={answers.projects[p.key]} onChange={(v) => answer('projects', p.key, v)}
                options={projects.map((l) => ({ value: l.id, label: `${l.name} (${l.key})` }))} />
            ))}
          </ul>
        </section>
      )}
      {check.warnings.length > 0 && <Alert tone="warning" title="To know">{check.warnings.map((w) => <p key={w}>{w}</p>)}</Alert>}
    </div>
  );
}

/** The names still waiting for an answer. */
function unanswered(check: ImportCheck, a: Answers) {
  return [
    ...check.connections.filter((c) => c.status !== 'found' && !(c.name in a.connections)).map((c) => c.name),
    ...check.personas.filter((p) => p.status !== 'found' && !(p.name in a.personas)).map((p) => p.name),
    ...check.projects.filter((p) => p.status !== 'found' && !(p.key in a.projects)).map((p) => p.key),
  ];
}

function target(c: Choice, where: string, w: ImportPreview): ImportTarget {
  const scope = where === 'host' || where === 'personal' ? where : 'project';
  return {
    scope, ...(scope === 'project' ? { projectId: where } : {}),
    ...(w.existing.length ? (c.clash === 'copy' ? { copy: true } : { versionOf: c.clash, copy: false }) : { copy: false }),
    skip: false,
  };
}

export function ImportDialog({ projects, onClose, onImported }: {
  projects: Project[]; onClose: () => void; onImported: (firstId: string) => void;
}) {
  const qc = useQueryClient();
  const [text, setText] = useState('');
  const [answers, setAnswers] = useState<Answers>(noAnswers);
  const [choices, setChoices] = useState<Choice[]>([]);
  const [result, setResult] = useState<ImportResult | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const check = useMutation({
    mutationFn: () => api.checkImport(text),
    onSuccess: (c) => { setAnswers(noAnswers()); setChoices(c.workflows.map(() => ({ where: '', clash: 'copy' }))); },
  });
  const report = check.data;
  const run = useMutation({
    mutationFn: () => {
      const mapping: ImportMapping = { connections: answers.connections, personas: answers.personas, projects: answers.projects };
      return api.importWorkflows({ text, mapping, targets: report!.workflows.map((w, i) => target(choices[i], whereOf(choices[i], w, report!, answers), w)) });
    },
    onSuccess: (r) => {
      void qc.invalidateQueries({ queryKey: ['workflows'] });
      if (r.warnings.length) setResult(r);
      else { onClose(); if (r.workflows[0]) onImported(r.workflows[0].id); }
    },
  });
  const edit = (t: string) => { setText(t); check.reset(); run.reset(); };
  const load = async (f: File | undefined) => { if (f) edit(await f.text()); };
  const onDrop = (e: DragEvent) => { e.preventDefault(); void load(e.dataTransfer.files[0]); };
  const waiting = report ? unanswered(report, answers) : [];
  const unplaced = report ? report.workflows.some((w, i) => !whereOf(choices[i], w, report, answers)) : false;
  const hasErrors = report?.workflows.some((w) => w.issues.some((i) => i.severity === 'error')) ?? false;

  if (result) {
    return (
      <Dialog open onOpenChange={(o) => { if (!o) onClose(); }} title="Imported" description="The new workflows are off: dry run them, then turn them on."
        footer={<Button variant="primary" onClick={() => { onClose(); if (result.workflows[0]) onImported(result.workflows[0].id); }}>Open</Button>}>
        <div className="flex flex-col gap-2">
          <ul className="text-sm">{result.workflows.map((w) => <li key={w.id}>{w.name}</li>)}</ul>
          <Alert tone="warning" title="Left to do">{result.warnings.map((w) => <p key={w}>{w}</p>)}</Alert>
        </div>
      </Dialog>
    );
  }
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }} dismissable={false} size="lg" title="Import workflows"
      description="Paste an export from another PDT, or a workflow document from a workflow's Text view."
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          {report ? (
            <Button variant="primary" disabled={waiting.length > 0 || unplaced || hasErrors} pending={run.isPending} onClick={() => run.mutate()}
              title={waiting.length ? `Map or leave unmapped: ${waiting.join(', ')}` : unplaced ? 'Pick where each applies' : undefined}>
              Import {report.workflows.length === 1 ? '1 workflow' : `${report.workflows.length} workflows`}
            </Button>
          ) : (
            <Button variant="primary" disabled={!text.trim()} pending={check.isPending} onClick={() => check.mutate()}>Check</Button>
          )}
        </>
      }>
      <div className="flex flex-col gap-3">
        <div onDragOver={(e) => e.preventDefault()} onDrop={onDrop} className="flex flex-col gap-1.5">
          <Textarea mono rows={report ? 4 : 12} aria-label="Export to import" placeholder="Paste the export here, or drop its file"
            value={text} onChange={(e) => edit(e.target.value)} className="max-h-[40vh] text-xs" />
          <div className="flex items-center gap-2">
            <Button size="sm" variant="secondary" onClick={() => picker.current?.click()}>Choose file</Button>
            <span className="text-meta text-muted-foreground">or drop it on the box</span>
          </div>
          <input ref={picker} type="file" accept=".json,application/json,text/plain" hidden aria-hidden tabIndex={-1}
            onChange={(e) => { void load(e.target.files?.[0]); e.target.value = ''; }} />
        </div>
        {check.error && <Alert tone="error">{check.error.message}</Alert>}
        {report && <Report check={report} answers={answers} setAnswers={setAnswers} choices={choices} setChoices={setChoices} projects={projects} />}
        {run.error && <Alert tone="error">{run.error.message}</Alert>}
      </div>
    </Dialog>
  );
}
