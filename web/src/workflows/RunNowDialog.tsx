// Run now's form (PDT-301): the inputs the workflow's Run now trigger declares, with their defaults, then its
// confirmation when one applies. A workflow that declares neither runs at once (WorkflowPage).
import { useState } from 'react';
import { Alert, Button, Dialog, Field, Input, SegmentedControl, Select, Switch } from '@ui';
import type { InputField } from '../api/gen/InputField';
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

export function RunNowDialog({ name, form, pending, error, onRun, onClose }: {
  name: string;
  form: RunForm;
  pending: boolean;
  error: string | null;
  onRun: (input: RunValues) => void;
  onClose: () => void;
}) {
  const [values, setValues] = useState<RunValues>(() => initialValues(form.fields));
  // The confirmation shown, once the form is filled in.
  const [asking, setAsking] = useState<string | null>(() => (form.fields.length ? null : confirmText(form.confirm, {})));
  const input = runInput(form.fields, values);
  const wrong = problem(form.fields, values);
  const next = () => {
    const text = confirmText(form.confirm, input);
    if (text) setAsking(text);
    else onRun(input);
  };
  const footer = asking !== null ? (
    <>
      {form.fields.length > 0 ? <Button variant="secondary" onClick={() => setAsking(null)}>Back</Button> : <Button variant="secondary" onClick={onClose}>Cancel</Button>}
      <Button variant="primary" pending={pending} onClick={() => onRun(input)}>Continue</Button>
    </>
  ) : (
    <>
      <Button variant="secondary" onClick={onClose}>Cancel</Button>
      <Button variant="primary" disabled={!!wrong} title={wrong ?? undefined} pending={pending} onClick={next}>Run now</Button>
    </>
  );
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }} title={`Run ${name}`} footer={footer} dismissable={false}>
      <div className="flex flex-col gap-3">
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
