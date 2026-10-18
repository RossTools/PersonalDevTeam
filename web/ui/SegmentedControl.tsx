import type { ReactNode } from 'react';
import type { Key } from 'react-aria-components';
import { cn } from '@ui/lib/utils';
import { ToggleGroup, ToggleGroupItem } from '@ui/primitives/toggle-group';

export interface SegmentedOption<V extends string = string> {
  value: V;
  label: ReactNode;
  /** Optional leading icon. */
  icon?: ReactNode;
  disabled?: boolean;
  /** Accessible name when `label` isn't text. */
  'aria-label'?: string;
}

export interface SegmentedControlProps<V extends string = string> {
  options: SegmentedOption<V>[];
  value: V;
  onChange: (value: V) => void;
  /** Names the group (e.g. "Source"). */
  'aria-label'?: string;
  'aria-labelledby'?: string;
  /** sm 24px, md 28px (default); 44px on touch screens. */
  size?: 'sm' | 'md';
  disabled?: boolean;
  className?: string;
}

/** One of a few views or modes (Folder | Clone URL, Setup | Output). Single selection; arrow keys move between segments. */
export function SegmentedControl<V extends string = string>({
  options,
  value,
  onChange,
  size = 'md',
  disabled,
  className,
  ...aria
}: SegmentedControlProps<V>) {
  return (
    <ToggleGroup
      {...aria}
      variant="segment"
      spacing={0.5}
      selectionMode="single"
      disallowEmptySelection
      selectedKeys={[value]}
      onSelectionChange={(keys: Set<Key>) => {
        const [k] = keys;
        if (k !== undefined && k !== value) onChange(String(k) as V);
      }}
      isDisabled={disabled}
      className={cn(size === 'sm' ? 'h-control-sm' : 'h-control-md', 'pointer-coarse:h-control-touch', className)}
    >
      {options.map((o) => (
        <ToggleGroupItem
          key={o.value}
          id={o.value}
          isDisabled={o.disabled}
          aria-label={o['aria-label']}
          className={cn('h-full min-w-0 rounded-sm px-2 pointer-coarse:h-full', size === 'sm' ? 'text-xs' : 'text-sm')}
        >
          {o.icon}
          {o.label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}
