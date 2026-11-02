import { cn } from '@ui/lib/utils';
import { Kbd as KbdPrimitive, KbdGroup } from '@ui/primitives/kbd';

export interface KbdProps {
  /** One key (`children`), or a sequence shown as separate keys (`keys={['Ctrl', ']']}`). */
  children?: React.ReactNode;
  keys?: string[];
  className?: string;
}

/** Touch screens have no keys to press: the hints hide there. */
const HIDE_ON_TOUCH = 'pointer-coarse:hidden';

/** A key hint (chooser, help, menus, tooltips). Hidden on touch screens. */
export function Kbd({ children, keys, className }: KbdProps) {
  if (keys) {
    return (
      <KbdGroup className={cn(HIDE_ON_TOUCH, className)}>
        {keys.map((k, i) => (
          <KbdPrimitive key={i}>{k}</KbdPrimitive>
        ))}
      </KbdGroup>
    );
  }
  return <KbdPrimitive className={cn(HIDE_ON_TOUCH, className)}>{children}</KbdPrimitive>;
}
