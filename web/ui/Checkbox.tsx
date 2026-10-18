import type { ReactNode } from 'react';
import { Checkbox as CheckboxPrimitive } from '@ui/primitives/checkbox';
import { useFieldControl } from './fieldContext';

export interface CheckboxProps {
  /** The label (or pass it as children). Without one, give `aria-label`. */
  label?: ReactNode;
  children?: ReactNode;
  checked?: boolean;
  defaultChecked?: boolean;
  indeterminate?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  name?: string;
  value?: string;
  id?: string;
  'aria-label'?: string;
  'aria-describedby'?: string;
  className?: string;
}

/** A checkbox with its label; `onChange(checked)`. Inside a Field it is described by the Field's hint and error. */
export function Checkbox({
  label,
  children,
  checked,
  defaultChecked,
  indeterminate,
  onChange,
  disabled,
  name,
  value,
  id,
  className,
  ...aria
}: CheckboxProps) {
  const field = useFieldControl({ id, disabled, ...aria });
  const text = label ?? children;
  return (
    <CheckboxPrimitive
      id={field.id}
      aria-label={aria['aria-label']}
      aria-labelledby={text || aria['aria-label'] ? undefined : field.labelledBy}
      aria-describedby={field['aria-describedby']}
      isInvalid={field['aria-invalid'] === true}
      isSelected={checked}
      defaultSelected={defaultChecked}
      isIndeterminate={indeterminate}
      onChange={onChange}
      isDisabled={field.disabled}
      name={name}
      value={value}
      className={className}
    >
      {text}
    </CheckboxPrimitive>
  );
}
