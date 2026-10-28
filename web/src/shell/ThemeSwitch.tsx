import { Monitor, Moon, Sun } from 'lucide-react';
import { SegmentedControl, type ThemePref } from '@ui';

const THEMES = [
  { value: 'system', label: 'System', icon: Monitor },
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
] as const;

/**
 * System | Light | Dark in one click (PDT-206): the side rail's footer (icons only; a column while the rail is
 * collapsed) and the phone drawer's (with names). The choice is `useThemePref`'s, kept in this browser.
 */
export function ThemeSwitch({ value, onChange, layout }: {
  value: ThemePref;
  onChange: (pref: ThemePref) => void;
  /** `column`: icons stacked (the collapsed rail); `icons`: icons in a row; `labels`: icons and names in a row. */
  layout: 'column' | 'icons' | 'labels';
}) {
  return (
    <SegmentedControl<ThemePref>
      aria-label="Theme"
      size="sm"
      orientation={layout === 'column' ? 'vertical' : 'horizontal'}
      className={layout === 'column' ? undefined : 'w-full *:flex-1'}
      options={THEMES.map(({ value: v, label, icon: Icon }) => ({
        value: v,
        icon: <Icon aria-hidden />,
        label: layout === 'labels' ? label : null,
        'aria-label': label,
      }))}
      value={value}
      onChange={onChange}
    />
  );
}
