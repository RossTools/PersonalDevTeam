import type { ComponentProps } from 'react';
import { Textarea as TextareaPrimitive } from '@ui/primitives/textarea';
import { useFieldControl } from './fieldContext';

export interface TextareaProps extends Omit<ComponentProps<'textarea'>, 'className'> {
  /** Monospace text. */
  mono?: boolean;
  className?: string;
}

/** Multi-line text input with the native textarea API; grows with its content. Inside a Field it is labelled by it. */
export function Textarea({ mono = false, className, ...props }: TextareaProps) {
  const { labelledBy, ...field } = useFieldControl(props);
  return <TextareaPrimitive autoComplete="off" {...props} {...field} aria-labelledby={labelledBy} className={mono ? `font-mono ${className ?? ''}` : className} />;
}
