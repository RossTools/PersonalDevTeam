// Run now's form (PDT-301): the project to run in, when the workflow's steps work on one and its scope gives none
// (default: the one in view), the inputs its Run now trigger declares, with their defaults, then its confirmation
// when one applies. A workflow that needs none of these runs at once (WorkflowPage).
import { useState } from 'react';
import { Alert, Button, Dialog, Field, Input, SegmentedControl, Select, Switch } from '@ui';
import type { InputField } from '../api/gen/InputField';
import type { Project } from '../api/gen/Project';
import { confirmText, fieldLabel, initialValues, problem, runInput, visibleFields, type RunForm, type RunValues } from './runInputs';

function Control({ f, value, onChange }: { f: InputField; value: unknown; onChange: (v: unknown) => void }) {
  const label = fieldLabel(f);
  switch (f.type) {
    case 'choice': {
      const options = (f.choices ?? []).map((c) => ({ value: c.value, label: c.label?.trim() ? c.label : c.value }));
      const v = typeof value === 'string' ? value : '';
      return options.length <= 4
        ? <SegmentedControl aria-label={label} options={options} value={v} onChange={onChange} className="self-start" />
        : <Select aria-label={label} options={options} value={v} onChange={onChange} />;
    }
    case 'boolean':
      return <Switch aria-label={label} checked={value === true} onChange={onChange} />;
    case 'number':
      return <Input aria-label={label} type="number" value={value == null ? '' : String(value)} onChange={(e) => onChange(e.target.value)} />;
    default:
      return <Input aria-label={label} value={value == null ? '' : String(value)} onChange={(e) => onChange(e.target.value)} />;
  }
}

export function RunNowDialog({ name, form, projects, project: inView, pending, error, onRun, onClose }: {
  name: string;
  form: RunForm;
  /** The projects to pick from, when it asks for one. */
  projects?: Project[];
  /** The project in view: the default pick. */
  project?: string | null;
  pending: boolean;
  error: string | null;
  onRun: (input: RunValues) => void;
  onClose: () => void;
}) {
  const [values, setValues] = useState<RunValues>(() => initialValues(form.fields));
  const choices = (projects ?? []).filter((p) => p.kind !== 'personal');
  const [projectId, setProjectId] = useState(() => choices.find((p) => p.id === inView)?.id ?? '');
  const hasForm = form.fields.length > 0 || !!projects;
  // The confirmation shown, once the form is filled in.
  const [asking, setAsking] = useState<string | null>(() => (hasForm ? null : confirmText(form.confirm, {})));
  const input = { ...runInput(form.fields, values), ...(projects && projectId ? { projectId } : {}) };
  const wrong = projects && !projectId ? 'Pick the project it runs in' : problem(form.fields, values);
  const next = () => {
    const text = confirmText(form.confirm, input);
    if (text) setAsking(text);
    else onRun(input);
  };
  const footer = asking !== null ? (
    <>
      {hasForm ? <Button variant="secondary" onClick={() => setAsking(null)}>Back</Button> : <Button variant="secondary" onClick={onClose}>Cancel</Button>}
      <Button variant="primary" pending={pending} onClick={() => onRun(input)}>Continue</Button>
    </>
  ) : (
    <>
      <Button variant="secondary" onClick={onClose}>Cancel</Button>
      <Button variant="primary" disabled={!!wrong} title={wrong ?? undefined} pending={pending} onClick={next}>Run now</Button>
    </>
  );
  return (
    <Dialog onClose={onClose} title={`Run ${name}`} footer={footer} dismissable={false}>
      <div className="flex flex-col gap-3">
        {asking === null && projects && (
          <Field label={<>Project<span className="text-muted-foreground"> *</span></>} hint="Its steps work on the project it runs in.">
            <Select aria-label="Project" placeholder="Pick a project" options={choices.map((p) => ({ value: p.id, label: p.name }))}
              value={projectId} onChange={setProjectId} />
          </Field>
        )}
        {asking !== null ? <p className="text-sm">{asking}</p> : visibleFields(form.fields, values).map((f) => (
          <Field key={f.id} label={<>{fieldLabel(f)}{f.required && <span className="text-muted-foreground"> *</span>}</>} hint={f.help || undefined}>
            <Control f={f} value={values[f.id]} onChange={(v) => setValues((old) => ({ ...old, [f.id]: v }))} />
          </Field>
        ))}
        {error && <Alert tone="error">{error}</Alert>}
      </div>
    </Dialog>
  );
}
