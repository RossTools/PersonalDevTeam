import { createContext, useContext } from 'react';

/** What a Field tells the control inside it: its ids, and whether it shows an error. */
export interface FieldContextValue {
  controlId: string;
  labelId: string;
  describedBy: string | undefined;
  invalid: boolean;
  disabled: boolean;
}

export const FieldContext = createContext<FieldContextValue | null>(null);

function joinIds(...ids: (string | undefined)[]) {
  const s = ids.filter(Boolean).join(' ');
  return s || undefined;
}

/**
 * Props a control takes from its Field: `id` (so the label's htmlFor finds it), `aria-labelledby` for controls a label
 * can't point at (Select, Switch), `aria-describedby` (hint + error) and `aria-invalid`. The control's own props win.
 */
export function useFieldControl(own: {
  id?: string;
  'aria-describedby'?: string;
  'aria-invalid'?: boolean | 'true' | 'false' | 'grammar' | 'spelling';
  'aria-label'?: string;
  'aria-labelledby'?: string;
  disabled?: boolean;
}) {
  const f = useContext(FieldContext);
  if (!f) {
    return {
      id: own.id,
      labelledBy: own['aria-labelledby'],
      'aria-describedby': own['aria-describedby'],
      'aria-invalid': own['aria-invalid'],
      disabled: own.disabled,
    };
  }
  return {
    // A control with its own aria-label is one of several in the Field (start/end ports): it keeps its own name and id.
    id: own.id ?? (own['aria-label'] ? undefined : f.controlId),
    labelledBy: own['aria-label'] ? own['aria-labelledby'] : (own['aria-labelledby'] ?? f.labelId),
    'aria-describedby': joinIds(own['aria-describedby'], f.describedBy),
    'aria-invalid': own['aria-invalid'] ?? (f.invalid || undefined),
    disabled: own.disabled ?? f.disabled,
  };
}
