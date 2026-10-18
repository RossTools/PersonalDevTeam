import type { ComponentProps } from 'react';
import { Spinner as SpinnerPrimitive } from '@ui/primitives/spinner';

export type SpinnerProps = Omit<ComponentProps<typeof SpinnerPrimitive>, 'ref'>;

/** Activity indicator: `role="status"` named `label` (default "Loading"); `decorative` hides it from assistive tech. */
export function Spinner(props: SpinnerProps) {
  return <SpinnerPrimitive {...props} />;
}
