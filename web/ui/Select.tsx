import type { Key } from 'react-aria-components';
import { cn } from 'cn';
import { Select as SelectRoot, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@ui/primitives/select';
import { useFieldControl } from './fieldContext';

export interface SelectOption<V extends string = string> {
  value: V;
  label: string;
  disabled?: boolean;
}

export interface SelectProps<V extends string = string> {
  options: SelectOption<V>[];
  /** The selected value; null/undefined/'' shows the placeholder. */
  value: V | null | undefined;
  onChange: (value: V) => void;
  /** Needed unless the Select sits in a Field (which labels it). */
  'aria-label'?: string;
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  placeholder?: string;
  /** sm 24px, md 28px (default); 44px on touch screens. */
  size?: 'sm' | 'md';
  disabled?: boolean;
  id?: string;
  name?: string;
  className?: string;
}

/** A pick-one list (React Aria Select: a button that opens a listbox; type-ahead, arrow keys, Escape). */
export function Select<V extends string = string>({
  options,
  value,
  onChange,
  placeholder = 'Select…',
  size = 'md',
  disabled,
  id,
  name,
  className,
  ...aria
}: SelectProps<V>) {
  const field = useFieldControl({ id, disabled, ...aria });
  const selected = value === undefined || value === '' ? null : value;
  return (
    <SelectRoot
      aria-label={aria['aria-label']}
      aria-labelledby={field.labelledBy}
      aria-describedby={field['aria-describedby']}
      isInvalid={field['aria-invalid'] === true || field['aria-invalid'] === 'true'}
      isDisabled={field.disabled}
      id={field.id}
      name={name}
      placeholder={placeholder}
      value={selected}
      onChange={(k: Key | null) => {
        if (k !== null) onChange(String(k) as V);
      }}
      disabledKeys={options.filter((o) => o.disabled).map((o) => o.value)}
      className={cn('min-w-0', className)}
    >
      <SelectTrigger size={size === 'sm' ? 'sm' : 'default'}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value} id={o.value} textValue={o.label}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </SelectRoot>
  );
}
