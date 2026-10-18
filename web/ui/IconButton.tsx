import type { ReactNode } from 'react';
import { renderButton, type ButtonBaseProps } from './Button';
import { Tooltip } from './Tooltip';

export interface IconButtonProps extends Omit<ButtonBaseProps, 'aria-label' | 'aria-pressed' | 'variant' | 'title'> {
  /** Required: the accessible name and the tooltip text. */
  label: string;
  /** The icon (a lucide icon). */
  children: ReactNode;
  /** Defaults to ghost. */
  variant?: ButtonBaseProps['variant'];
  /** For toggles: sets aria-pressed and the selected fill. */
  pressed?: boolean;
  /** Extra tooltip content after the label, e.g. a `<Kbd>` hint. */
  shortcut?: ReactNode;
  /** Hide the tooltip (the label still names the button). */
  noTooltip?: boolean;
}

/** An icon-only button: square (24px sm, 28px md, 44px on touch), named by `label`, which its tooltip shows. */
export function IconButton({ label, pressed, shortcut, noTooltip = false, variant = 'ghost', ...rest }: IconButtonProps) {
  const button = renderButton({ ...rest, variant, icon: true, 'aria-label': label, 'aria-pressed': pressed });
  if (noTooltip) return button;
  return (
    <Tooltip content={shortcut ? <>{label}{shortcut}</> : label}>
      {button}
    </Tooltip>
  );
}
