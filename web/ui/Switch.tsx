import type { ReactNode } from 'react';
import { Switch as SwitchPrimitive } from '@ui/primitives/switch';
import { useFieldControl } from './fieldContext';

export interface SwitchProps {
  /** The label (or pass it as children). Inside a Field without one, the Field's label names it. */
  label?: ReactNode;
  children?: ReactNode;
  checked?: boolean;
  defaultChecked?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  size?: 'sm' | 'md';
  name?: string;
  id?: string;
  'aria-label'?: string;
  'aria-describedby'?: string;
  className?: string;
}

/** An on/off setting that applies at once (spec §6.1); `onChange(checked)`. */
export function Switch({
  label,
  children,
  checked,
  defaultChecked,
  onChange,
  disabled,
  size = 'md',
  name,
  id,
  className,
  ...aria
}: SwitchProps) {
  const field = useFieldControl({ id, disabled, ...aria });
  const text = label ?? children;
  return (
    <SwitchPrimitive
      id={field.id}
      aria-label={aria['aria-label']}
      aria-labelledby={text || aria['aria-label'] ? undefined : field.labelledBy}
      aria-describedby={field['aria-describedby']}
      isSelected={checked}
      defaultSelected={defaultChecked}
      onChange={onChange}
      isDisabled={field.disabled}
      size={size === 'sm' ? 'sm' : 'default'}
      name={name}
      className={className}
    >
      {text}
    </SwitchPrimitive>
  );
}
