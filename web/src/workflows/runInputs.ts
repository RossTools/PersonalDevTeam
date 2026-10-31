// Run now's form (PDT-301): the inputs a workflow's Run now trigger declares, as the server reads them
// (pdt-core workflow::inputs): defaults filled in, a field shown only while its showIf holds (and the field it names is
// shown), an empty field left out. The server checks the input again when the run starts.
import type { InputField } from '../api/gen/InputField';
import type { RunConfirm } from '../api/gen/RunConfirm';
import type { ShowIf } from '../api/gen/ShowIf';
import type { WorkflowDoc } from '../api/gen/WorkflowDoc';

export type RunValues = Record<string, unknown>;

export interface RunForm {
  fields: InputField[];
  confirm: RunConfirm | null;
}

/** The Run now trigger's declared inputs and confirmation; null when it has neither (Run now starts at once). */
export function runForm(doc: WorkflowDoc): RunForm | null {
  const node = doc.nodes.find((n) => n.kind === 'manual');
  const config = (node?.config ?? {}) as { inputs?: InputField[]; confirm?: RunConfirm | null };
  const fields = Array.isArray(config.inputs) ? config.inputs : [];
  const confirm = config.confirm?.text?.trim() ? config.confirm : null;
  return fields.length || confirm ? { fields, confirm } : null;
}

export const fieldLabel = (f: InputField) => (f.label?.trim() ? f.label : f.id);

/** What the form starts with: each field's default, else empty. */
export function initialValues(fields: InputField[]): RunValues {
  return Object.fromEntries(fields.map((f) => [f.id, f.default ?? (f.type === 'boolean' ? false : '')]));
}

const empty = (v: unknown) => v === undefined || v === null || v === '';

const holds = (s: ShowIf | undefined, values: RunValues) => !s || (empty(values[s.field]) ? '' : values[s.field]) === s.equals;

/** The fields shown for these values. */
export function visibleFields(fields: InputField[], values: RunValues): InputField[] {
  const shown: InputField[] = [];
  for (const f of fields) {
    if (f.showIf && !shown.some((s) => s.id === f.showIf!.field)) continue;
    if (holds(f.showIf, values)) shown.push(f);
  }
  return shown;
}

/** The run's input: the shown fields that have a value. */
export function runInput(fields: InputField[], values: RunValues): RunValues {
  return Object.fromEntries(visibleFields(fields, values).filter((f) => !empty(values[f.id]))
    .map((f) => [f.id, f.type === 'number' ? Number(values[f.id]) : values[f.id]]));
}

/** The first shown field that is required and empty, or a number field that isn't one: what to fix before starting. */
export function problem(fields: InputField[], values: RunValues): string | null {
  for (const f of visibleFields(fields, values)) {
    const v = values[f.id];
    if (f.required && empty(v)) return `${fieldLabel(f)} is required`;
    if (f.type === 'number' && !empty(v) && Number.isNaN(Number(v))) return `${fieldLabel(f)} must be a number`;
  }
  return null;
}

/** The confirmation for this input, its `{{trigger.<id>}}` filled in; null when it doesn't apply. */
export function confirmText(confirm: RunConfirm | null, input: RunValues): string | null {
  if (!confirm || !holds(confirm.showIf, input)) return null;
  return confirm.text.replace(/\{\{\s*trigger\.([\w.]+)\s*\}\}/g, (_, k: string) => {
    const v = input[k];
    return empty(v) ? '' : String(v);
  });
}
