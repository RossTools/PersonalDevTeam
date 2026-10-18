import { Kbd as KbdPrimitive, KbdGroup } from '@ui/primitives/kbd';

export interface KbdProps {
  /** One key (`children`), or a sequence shown as separate keys (`keys={['Ctrl', ']']}`). */
  children?: React.ReactNode;
  keys?: string[];
  className?: string;
}

/** A key hint (chooser, help, menus, tooltips). */
export function Kbd({ children, keys, className }: KbdProps) {
  if (keys) {
    return (
      <KbdGroup className={className}>
        {keys.map((k, i) => (
          <KbdPrimitive key={i}>{k}</KbdPrimitive>
        ))}
      </KbdGroup>
    );
  }
  return <KbdPrimitive className={className}>{children}</KbdPrimitive>;
}
