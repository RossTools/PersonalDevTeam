import type { ReactNode, Ref } from 'react';
import { Radio, RadioGroup } from 'react-aria-components';
import { cn } from '@ui/lib/utils';
import { pressableChild } from '@ui/lib/pressableChild';

const card = (size: 'sm' | 'md') =>
  cn(
    'flex min-w-0 flex-col gap-1 rounded-lg border border-border bg-background text-left outline-none select-none hover:border-muted-foreground',
    'focus-visible:ring-2 focus-visible:ring-ring',
    size === 'sm' ? 'px-2.5 py-2' : 'p-3',
  );
const chosen = 'border-primary bg-accent hover:border-primary';
const off = 'cursor-not-allowed opacity-50 hover:border-border';

function Body({ title, description, children }: { title?: ReactNode; description?: ReactNode; children?: ReactNode }) {
  return (
    <>
      {title && <span className="text-sm font-medium">{title}</span>}
      {description && <span className="line-clamp-2 text-xs text-muted-foreground">{description}</span>}
      {children}
    </>
  );
}

export interface ChoiceCardProps {
  title?: ReactNode;
  /** A muted line or two under the title. */
  description?: ReactNode;
  /** More under the description (badges, a status line). */
  children?: ReactNode;
  selected?: boolean;
  /**
   * How `selected` is said: `pressed` (default; a pick among cards, `aria-pressed`) or `current` (the card for what's
   * shown now, `aria-current`).
   */
  selection?: 'pressed' | 'current';
  /** A dashed edge: the "start from nothing" card among templates. */
  dashed?: boolean;
  /** sm: 10px × 8px padding (a grid of times); md (default) 12px. */
  size?: 'sm' | 'md';
  disabled?: boolean;
  onClick?: () => void;
  'aria-label'?: string;
  className?: string;
  ref?: Ref<HTMLButtonElement>;
}

/**
 * A card to pick: a title, a description and whatever goes under them, bordered, with the accent edge and fill when
 * selected. Wrap it in a `ContextMenu` for a menu on right-click. For one of a few options in a form, use
 * `ChoiceCardGroup` (a radio group) instead.
 */
export function ChoiceCard({
  title, description, children, selected = false, selection = 'pressed', dashed, size = 'md', disabled, onClick, className, ref, ...aria
}: ChoiceCardProps) {
  // Pressable, so a ContextMenu around the card opens on right-click and long press.
  return pressableChild(
    <button
      ref={ref}
      type="button"
      {...aria}
      aria-pressed={selection === 'pressed' ? selected : undefined}
      aria-current={selection === 'current' && selected ? true : undefined}
      disabled={disabled}
      onClick={onClick}
      // `className` before the selected look, so a card's own fill never hides it.
      className={cn(card(size), className, dashed && 'border-dashed', selected && chosen, disabled && off)}
    >
      <Body title={title} description={description}>{children}</Body>
    </button>,
  );
}

export interface ChoiceOption<V extends string = string> {
  value: V;
  title: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
  /** The option's name when `title` isn't plain text. */
  'aria-label'?: string;
}

/** One of a few options as cards side by side (the default agent): a radio group, arrow keys move and pick. */
export function ChoiceCardGroup<V extends string = string>({ options, value, onChange, size = 'md', className, ...aria }: {
  options: ChoiceOption<V>[];
  value: V | null;
  onChange: (value: V) => void;
  'aria-label': string;
  size?: 'sm' | 'md';
  /** On the group: the grid it lays the cards out in. */
  className?: string;
}) {
  return (
    <RadioGroup
      {...aria}
      value={value}
      onChange={(v) => onChange(v as V)}
      className={cn('grid grid-cols-1 gap-2 sm:grid-cols-3', className)}
    >
      {options.map((o) => (
        <Radio
          key={o.value}
          value={o.value}
          isDisabled={o.disabled}
          aria-label={o['aria-label']}
          // The focus is on the hidden input: React Aria marks the card with data-focus-visible.
          className={({ isSelected, isDisabled }) => cn(card(size), 'cursor-default data-focus-visible:ring-2 data-focus-visible:ring-ring', isSelected && chosen, isDisabled && off)}
        >
          <Body title={o.title} description={o.description} />
        </Radio>
      ))}
    </RadioGroup>
  );
}
