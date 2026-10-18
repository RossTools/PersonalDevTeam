import { useEffect, useRef, type ReactNode, type RefObject } from 'react';
import { Search, X } from 'lucide-react';
import { cn } from './lib/utils';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from './primitives/input-group';
import { Kbd as KbdPrimitive } from './primitives/kbd';

/** True when a key press belongs to whatever has focus (a text field, an editor, a terminal), not to page shortcuts. */
function typingInto(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target.closest('input, textarea, select, [role="textbox"], .xterm')) return true;
  return false;
}

/**
 * Focuses `ref` when `/` is pressed outside a text field or terminal (no modifiers). Idempotent; off when `enabled`
 * is false (e.g. while the view is hidden).
 */
export function useSlashToFocus(ref: RefObject<HTMLInputElement | null>, enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return;
      if (typingInto(e.target)) return;
      const input = ref.current;
      if (!input) return;
      e.preventDefault();
      input.focus();
      input.select();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [ref, enabled]);
}

export interface FilterBarProps {
  search: string;
  onSearchChange: (value: string) => void;
  /** Placeholder and accessible name of the search box. */
  searchLabel?: string;
  /** The search input, for the parent to focus it. */
  searchRef?: RefObject<HTMLInputElement | null>;
  /** `/` anywhere outside a text field focuses search (default true). */
  slashToFocus?: boolean;
  /** Filter chips (stage, priority, label menus) after the search box. */
  children?: ReactNode;
  /** Pinned to the end: group, sort, view options. */
  trailing?: ReactNode;
  className?: string;
}

/**
 * The filter row above a list: a search box (`/` focuses it, Escape clears then leaves it), filter chips and a trailing
 * slot. Fixed height on desktop (h-10, the same as `BulkActionBar`, which replaces it in place); wraps on phones.
 */
export function FilterBar({
  search,
  onSearchChange,
  searchLabel = 'Search',
  searchRef,
  slashToFocus = true,
  children,
  trailing,
  className,
}: FilterBarProps) {
  const own = useRef<HTMLInputElement>(null);
  const ref = searchRef ?? own;
  useSlashToFocus(ref, slashToFocus);
  return (
    <div
      role="search"
      data-slot="filter-bar"
      className={cn('flex min-h-10 flex-wrap items-center gap-x-2 gap-y-1.5 px-3 py-1.5 md:h-10 md:flex-nowrap md:py-0', className)}
    >
      <InputGroup className="w-full shrink-0 md:w-56">
        <InputGroupAddon>
          <Search aria-hidden />
        </InputGroupAddon>
        <InputGroupInput
          ref={ref}
          type="search"
          value={search}
          aria-label={searchLabel}
          aria-keyshortcuts={slashToFocus ? '/' : undefined}
          placeholder={searchLabel}
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => onSearchChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== 'Escape') return;
            if (search) onSearchChange('');
            else e.currentTarget.blur();
            e.preventDefault();
          }}
          className="[&::-webkit-search-cancel-button]:appearance-none"
        />
        <InputGroupAddon align="inline-end">
          {search ? (
            <InputGroupButton size="icon-xs" aria-label="Clear search" onPress={() => { onSearchChange(''); ref.current?.focus(); }}>
              <X aria-hidden />
            </InputGroupButton>
          ) : (
            slashToFocus && <KbdPrimitive aria-hidden className="pointer-coarse:hidden">/</KbdPrimitive>
          )}
        </InputGroupAddon>
      </InputGroup>
      {children != null && <div className="flex min-w-0 flex-wrap items-center gap-1.5 md:flex-nowrap">{children}</div>}
      {trailing != null && <div className="ml-auto flex shrink-0 items-center gap-1.5">{trailing}</div>}
    </div>
  );
}
