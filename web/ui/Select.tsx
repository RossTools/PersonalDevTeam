import { Autocomplete, useFilter, type Key } from 'react-aria-components';
import { cn } from '@ui/lib/utils';
import {
  Select as SelectRoot,
  SelectContent,
  SelectEmpty,
  SelectInput,
  SelectItem,
  SelectList,
  SelectPopover,
  SelectTrigger,
  SelectValue,
} from '@ui/primitives/select';
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
  /** A borderless chip in muted text that fills on hover (toolbars, e.g. a prompt box's pickers). */
  quiet?: boolean;
  disabled?: boolean;
  /** A filter field at the top of the list: typing narrows it to the options that contain the text (long lists). */
  searchable?: boolean;
  id?: string;
  name?: string;
  className?: string;
}

/** A pick-one list (React Aria Select: a button that opens a listbox; type-ahead, arrow keys, Escape; `searchable` adds a filter field). */
export function Select<V extends string = string>({
  options,
  value,
  onChange,
  placeholder = 'Select…',
  size = 'md',
  quiet = false,
  disabled,
  searchable = false,
  id,
  name,
  className,
  ...aria
}: SelectProps<V>) {
  const field = useFieldControl({ id, disabled, ...aria });
  const selected = value === undefined || value === '' ? null : value;
  const { contains } = useFilter({ sensitivity: 'base' });
  const items = options.map((o) => (
    <SelectItem key={o.value} id={o.value} textValue={o.label}>
      {o.label}
    </SelectItem>
  ));
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
      <SelectTrigger
        size={size === 'sm' ? 'sm' : 'default'}
        className={cn(quiet && 'w-auto border-transparent bg-transparent text-muted-foreground hover:bg-muted hover:text-foreground')}
      >
        <SelectValue />
      </SelectTrigger>
      {searchable ? (
        <SelectPopover className="flex max-h-80 flex-col">
          <Autocomplete filter={contains}>
            <SelectInput aria-label="Filter" placeholder="Filter…" className="pb-1" />
            <SelectList className="min-h-0 flex-1" renderEmptyState={() => <SelectEmpty>No matches</SelectEmpty>}>
              {items}
            </SelectList>
          </Autocomplete>
        </SelectPopover>
      ) : (
        <SelectContent>{items}</SelectContent>
      )}
    </SelectRoot>
  );
}
