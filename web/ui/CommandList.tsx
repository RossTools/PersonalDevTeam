import type { ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { useFilter, type AutocompleteProps } from 'react-aria-components';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem as CommandItemPrimitive,
  CommandList as CommandListPrimitive,
  CommandShortcut,
} from './primitives/command';
import { Kbd } from './Kbd';

export interface CommandItem {
  id: string;
  label: string;
  /** Secondary text after the label, muted (a tab's workspace). Also matched by the filter. */
  detail?: string;
  /** Items with the same group are listed together under it, groups in order of first appearance. */
  group?: string;
  /** Shown as a key hint ("⌘K", "Ctrl+] k"). */
  shortcut?: string;
  icon?: ReactNode;
  /** Extra words the filter matches (a branch's project, a session's agent). */
  keywords?: string[];
  disabled?: boolean;
}

export interface CommandListProps {
  items: CommandItem[];
  /** Called with the chosen item's id (click, tap or Enter). */
  onAction: (id: string) => void;
  /** Accessible name of the list ("Commands", "Places"). */
  label: string;
  placeholder?: string;
  /** Controlled filter text. */
  query?: string;
  onQueryChange?: (query: string) => void;
  /** Shown inside the list when nothing matches. */
  emptyText?: ReactNode;
  /** Focus the filter field on mount (default true). */
  autoFocus?: boolean;
  className?: string;
}

function groupItems(items: CommandItem[]) {
  const loose: CommandItem[] = [];
  const groups = new Map<string, CommandItem[]>();
  for (const it of items) {
    if (!it.group) loose.push(it);
    else groups.set(it.group, [...(groups.get(it.group) ?? []), it]);
  }
  return { loose, groups: [...groups] };
}

/**
 * A filterable, keyboard-driven list: type to filter (label, detail and keywords), Up/Down to move, Enter to choose. Usable
 * inline, in a Popover or a Dialog; the command palette and PlacePicker are built on it.
 */
export function CommandList({
  items,
  onAction,
  label,
  placeholder = 'Search…',
  query,
  onQueryChange,
  emptyText = 'No results',
  autoFocus = true,
  className,
}: CommandListProps) {
  const { contains } = useFilter({ sensitivity: 'base' });
  const filter: AutocompleteProps<object>['filter'] = (text, input, node) => {
    const item = node.value as CommandItem | null;
    return contains(text, input) || (!!item?.detail && contains(item.detail, input)) || !!item?.keywords?.some((k) => contains(k, input));
  };
  const { loose, groups } = groupItems(items);
  const renderItem = (it: CommandItem) => (
    <CommandItemPrimitive key={it.id} id={it.id} value={it} textValue={it.label} isDisabled={it.disabled}>
      {it.icon}
      <span className="min-w-0 flex-1 truncate">
        {it.label}
        {it.detail && (
          <>
            {' '}
            <span className="ml-1 text-meta text-muted-foreground">{it.detail}</span>
          </>
        )}
      </span>
      {it.shortcut && (
        <CommandShortcut className="tracking-normal">
          <Kbd>{it.shortcut}</Kbd>
        </CommandShortcut>
      )}
    </CommandItemPrimitive>
  );
  return (
    <Command
      filter={filter}
      {...(query === undefined ? {} : { inputValue: query })}
      onInputChange={onQueryChange}
      className={cn('rounded-md! bg-transparent', className)}
    >
      <CommandInput placeholder={placeholder} autoFocus={autoFocus} />
      <CommandListPrimitive
        aria-label={label}
        onAction={(key) => onAction(String(key))}
        renderEmptyState={() => <CommandEmpty className="py-4 text-xs text-muted-foreground">{emptyText}</CommandEmpty>}
        className="mt-1"
      >
        {loose.map(renderItem)}
        {groups.map(([name, its]) => (
          <CommandGroup key={name} id={`group:${name}`} heading={name} className="p-0 pt-1">
            {its.map(renderItem)}
          </CommandGroup>
        ))}
      </CommandListPrimitive>
    </Command>
  );
}
