import type { ComponentProps } from 'react';
import { Input as InputPrimitive } from '@ui/primitives/input';
import { useFieldControl } from './fieldContext';

export interface InputProps extends Omit<ComponentProps<'input'>, 'size' | 'className'> {
  /** sm 24px, md 28px (default); 44px on touch screens. */
  size?: 'sm' | 'md';
  /** Monospace text (branches, paths, commands). */
  mono?: boolean;
  className?: string;
}

/** Single-line text input with the native input API (`value`, `onChange(e)`, …). Inside a Field it is labelled and described by it. */
export function Input({ size = 'md', mono = false, className, ...props }: InputProps) {
  const { labelledBy, ...field } = useFieldControl(props);
  // PDT's fields are names, keys, paths and commands, never personal data: no browser autofill suggestions unless a
  // caller asks for them.
  return <InputPrimitive autoComplete="off" {...props} {...field} aria-labelledby={labelledBy} size={size} className={mono ? `font-mono ${className ?? ''}` : className} />;
}
