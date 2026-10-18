import { useId, type ReactNode } from 'react';
import { cn } from 'cn';
import { Label } from '@ui/primitives/label';
import { FieldDescription, FieldError } from '@ui/primitives/field';
import { FieldContext } from './fieldContext';

export interface FieldProps {
  label: ReactNode;
  /** The control: an @ui Input, Textarea, Select, Switch, Checkbox or SegmentedControl. */
  children: ReactNode;
  /** Help text under the control (wired with aria-describedby). */
  hint?: ReactNode;
  /** Error text; marks the control aria-invalid and is announced. */
  error?: ReactNode;
  /** One-line settings row: label (and hint) on the left, control on the right. Stacks on narrow screens. */
  inline?: boolean;
  disabled?: boolean;
  /** Id for the control (default: generated). */
  id?: string;
  className?: string;
}

/** Label + control + hint + error, one layout. */
export function Field({ label, children, hint, error, inline = false, disabled = false, id, className }: FieldProps) {
  const auto = useId();
  const controlId = id ?? `${auto}-control`;
  const labelId = `${auto}-label`;
  const hintId = hint ? `${auto}-hint` : undefined;
  const errorId = error ? `${auto}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(' ') || undefined;
  const ctx = { controlId, labelId, describedBy, invalid: !!error, disabled };

  const labelEl = (
    <Label id={labelId} htmlFor={controlId} className="text-xs font-medium text-foreground">
      {label}
    </Label>
  );
  const hintEl = hint && (
    <FieldDescription id={hintId} className="text-meta text-muted-foreground">
      {hint}
    </FieldDescription>
  );
  const errorEl = error && (
    <FieldError id={errorId} className="text-meta text-destructive">
      {error}
    </FieldError>
  );

  return (
    <FieldContext.Provider value={ctx}>
      {inline ? (
        <div
          data-slot="field"
          data-disabled={disabled || undefined}
          className={cn('flex flex-col gap-1.5 py-1 sm:flex-row sm:items-center sm:justify-between sm:gap-4', className)}
        >
          <div className="flex min-w-0 flex-col gap-0.5">
            {labelEl}
            {hintEl}
          </div>
          <div className="flex shrink-0 flex-col gap-1 sm:items-end">
            {children}
            {errorEl}
          </div>
        </div>
      ) : (
        <div data-slot="field" data-disabled={disabled || undefined} className={cn('flex flex-col gap-1', className)}>
          {labelEl}
          {children}
          {hintEl}
          {errorEl}
        </div>
      )}
    </FieldContext.Provider>
  );
}
