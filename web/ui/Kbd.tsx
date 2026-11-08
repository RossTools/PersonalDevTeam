import { Fragment } from 'react';
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

/** A key hint in a `KeyHints` line: the keys, then what they do. */
export type KeyHint = [keys: string[], does: string];

/**
 * The keys a view answers to, in a line: "↑ ↓ move · Enter import · Esc close" (a dialog's footer, a composer's
 * meta). Muted, meta-sized text; hidden on touch screens, and a dialog footer that holds only this hides with it.
 */
export function KeyHints({ hints, className }: { hints: KeyHint[]; className?: string }) {
  return (
    <span data-slot="key-hints" className={cn('flex flex-wrap items-center gap-1 text-meta text-muted-foreground', HIDE_ON_TOUCH, className)}>
      {hints.map(([keys, does], i) => (
        <Fragment key={i}>
          {i > 0 && ' · '}
          {keys.map((k) => <KbdPrimitive key={k}>{k}</KbdPrimitive>)} {does}
        </Fragment>
      ))}
    </span>
  );
}
