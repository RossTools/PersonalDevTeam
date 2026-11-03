import { useId, type ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { Checkbox } from './Checkbox';

export interface CheckboxPick {
  id: string;
  name: ReactNode;
}

export interface CheckboxPicksProps {
  label: ReactNode;
  /** Under the checkboxes: what none ticked means ("Every project when none is ticked."). */
  hint?: ReactNode;
  options: CheckboxPick[];
  /** The ticked ids, in the order they were ticked. */
  value: string[];
  onChange: (ids: string[]) => void;
  className?: string;
}

/**
 * A labelled group of checkboxes that picks some ids from a list (who may use a secret, which projects get a server).
 * Renders nothing when there is nothing to pick.
 */
export function CheckboxPicks({ label, hint, options, value, onChange, className }: CheckboxPicksProps) {
  const id = useId();
  if (!options.length) return null;
  const toggle = (pick: string, on: boolean) => onChange(on ? [...value, pick] : value.filter((x) => x !== pick));
  return (
    <div
      role="group"
      aria-labelledby={`${id}-label`}
      aria-describedby={hint ? `${id}-hint` : undefined}
      className={cn('flex flex-col gap-1.5', className)}
    >
      <span id={`${id}-label`} className="text-xs font-medium text-foreground">{label}</span>
      <div className="flex flex-wrap gap-x-4 gap-y-1">
        {options.map((o) => (
          <Checkbox key={o.id} checked={value.includes(o.id)} onChange={(on) => toggle(o.id, on)}>{o.name}</Checkbox>
        ))}
      </div>
      {hint && <p id={`${id}-hint`} className="text-meta text-muted-foreground">{hint}</p>}
    </div>
  );
}
