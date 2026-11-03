import { useRef, type ReactNode } from 'react';
import { GripVertical } from 'lucide-react';
import {
  Button as AriaButton,
  DropIndicator,
  GridList,
  GridListItem,
  useDragAndDrop,
  type DroppableCollectionDropEvent,
  type GridListItemProps,
  type Key,
  type Selection,
} from 'react-aria-components';
import { cn } from './lib/utils';
import { RowMenus, openFocusedRowMenu, useRowMenu, type MenuPoint } from './lib/rowMenu';

// ---------------------------------------------------------------------------------------------------------------
// Drag data: text/plain `pdt:<kind>:<id>` (Android Chrome drops custom MIME types; touch spike, 2026-10-17).

const PREFIX = 'pdt:';

/** The drag payload for one item: `pdt:<kind>:<id>`. */
export function dragData(kind: string, id: string): string {
  return `${PREFIX}${kind}:${id}`;
}

/** Reads a `pdt:<kind>:<id>` payload; anything else (stray text) is null. Ids may contain colons. */
export function parseDragData(text: string): { kind: string; id: string } | null {
  if (!text.startsWith(PREFIX)) return null;
  const rest = text.slice(PREFIX.length);
  const i = rest.indexOf(':');
  if (i <= 0 || i === rest.length - 1) return null;
  return { kind: rest.slice(0, i), id: rest.slice(i + 1) };
}

/** Where a drop landed: before/after a row, on a row, or on the list itself (`target` null, e.g. an empty list). */
export interface ListDropEvent {
  kind: string;
  id: string;
  target: { id: string; position: 'before' | 'after' | 'on' } | null;
}

// ---------------------------------------------------------------------------------------------------------------
// Row menus: right-click, long press (touch) and Shift+F10 / the Menu key all call a row's `onMenu`.

export type { MenuPoint } from './lib/rowMenu';

// ---------------------------------------------------------------------------------------------------------------
// List

export interface ListProps<T extends object> {
  'aria-label': string;
  /** Dynamic rows: `items` plus a render function as `children`. Each item needs an `id` (or `key`). */
  items?: Iterable<T>;
  /** With `items`: values the row render function reads from outside the items (current tab, activity…); rows re-render
   *  when they change. React Aria caches dynamic rows by item otherwise. */
  dependencies?: readonly unknown[];
  children: ReactNode | ((item: T) => ReactNode);
  /** `none` (default): rows act (`onAction`), the current one is marked with `ListRow current`. `single`: click selects. */
  selectionMode?: 'none' | 'single';
  selectedKey?: string | null;
  defaultSelectedKey?: string;
  onSelectionChange?: (id: string | null) => void;
  /** Click, tap or Enter on a row (with `single` selection: double click or Enter). */
  onAction?: (id: string) => void;
  /** Shown inside the list when it has no rows (an `EmptyState`). */
  renderEmpty?: () => ReactNode;
  /** Makes rows draggable (they get a drag handle); each carries text/plain `pdt:<dragKind>:<row id>`. */
  dragKind?: string;
  /** Kinds this list takes when something is dropped on it (default: `[dragKind]`). */
  acceptKinds?: string[];
  /** Makes the list a drop target. Payloads that aren't `pdt:<accepted kind>:<id>` are ignored. */
  onDrop?: (e: ListDropEvent) => void;
  className?: string;
  id?: string;
}


/** A keyboard-navigable list of `ListRow`s (React Aria GridList): Up/Down, Home/End, typeahead, Enter. */
export function List<T extends object>({
  items,
  children,
  selectionMode = 'none',
  selectedKey,
  defaultSelectedKey,
  onSelectionChange,
  onAction,
  renderEmpty,
  dragKind,
  acceptKinds,
  onDrop,
  className,
  ...rest
}: ListProps<T>) {
  const menus = useRef(new Map<string, (p: MenuPoint) => void>()).current;
  const accepted = acceptKinds ?? (dragKind ? [dragKind] : []);
  const { dragAndDropHooks } = useDragAndDrop<T>({
    getItems: dragKind ? (keys) => [...keys].map((k) => ({ 'text/plain': dragData(dragKind, String(k)) })) : undefined,
    getAllowedDropOperations: () => ['move'],
    acceptedDragTypes: onDrop ? ['text/plain'] : undefined,
    getDropOperation: () => 'move',
    onDrop: onDrop
      ? async (e: DroppableCollectionDropEvent) => {
          for (const item of e.items) {
            if (item.kind !== 'text' || !item.types.has('text/plain')) continue;
            const parsed = parseDragData(await item.getText('text/plain'));
            if (!parsed || !accepted.includes(parsed.kind)) continue;
            const t = e.target;
            onDrop({
              ...parsed,
              target: t.type === 'item' ? { id: String(t.key), position: t.dropPosition } : null,
            });
          }
        }
      : undefined,
    renderDropIndicator: (target) => (
      <DropIndicator target={target} className="mx-2 h-0 outline-1 outline-primary data-drop-target:outline-2" />
    ),
  });
  const selection =
    selectionMode === 'single'
      ? {
          selectionMode: 'single' as const,
          selectionBehavior: 'replace' as const,
          ...(selectedKey !== undefined
            ? { selectedKeys: selectedKey === null ? [] : [selectedKey] }
            : { defaultSelectedKeys: defaultSelectedKey ? [defaultSelectedKey] : [] }),
          onSelectionChange: (keys: Selection) => {
            if (!onSelectionChange) return;
            const first = keys === 'all' ? null : [...keys][0];
            onSelectionChange(first == null ? null : String(first));
          },
        }
      : { selectionMode: 'none' as const };

  return (
    <RowMenus.Provider value={menus}>
      {/* Shift+F10 / the Menu key on the focused row open its menu (rows can't take key handlers themselves). */}
      <div
        className="contents"
        onKeyDown={(e) => openFocusedRowMenu(e, menus)}
      >
        <GridList<T>
          {...rest}
          {...selection}
          items={items}
          dependencies={rest.dependencies ? [...rest.dependencies] : undefined}
          onAction={onAction ? (key: Key) => onAction(String(key)) : undefined}
          renderEmptyState={renderEmpty ? () => renderEmpty() : undefined}
          dragAndDropHooks={dragKind || onDrop ? dragAndDropHooks : undefined}
          className={cn('flex flex-col gap-0.5 outline-none', className)}
        >
          {children}
        </GridList>
      </div>
    </RowMenus.Provider>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// ListRow

export interface ListRowProps
  extends Omit<GridListItemProps, 'children' | 'className' | 'id' | 'textValue' | 'isDisabled' | 'style'> {
  id: string;
  title: ReactNode;
  /** Text for typeahead and the row's accessible name; defaults to `title` when it is a string. */
  textValue?: string;
  /** A second, quieter line (where it is, branch…), aligned under the title. */
  secondary?: ReactNode;
  /** More under the secondary line, wrapping instead of cut (a two-line preview, a meta line). */
  details?: ReactNode;
  /** Before the title: a status dot, an icon. */
  leading?: ReactNode;
  /** After the title: a state label, a count, a key. */
  trailing?: ReactNode;
  /** The row on screen (the open workspace, the focused tab): the soft accent fill and a bar on its left edge. */
  current?: boolean;
  /** Unseen: the title in semibold. */
  bold?: boolean;
  disabled?: boolean;
  /** Opens the row's menu: right-click, long press on touch, Shift+F10 or the Menu key. */
  onMenu?: (at: MenuPoint) => void;
  className?: string;
}

/** The selected/hover row of every list: leading slot, title, secondary line, trailing slot, drag handle. */
export function ListRow({
  id,
  title,
  textValue,
  secondary,
  details,
  leading,
  trailing,
  current = false,
  bold = false,
  disabled,
  onMenu,
  className,
  ...rest
}: ListRowProps) {
  const text = textValue ?? (typeof title === 'string' ? title : undefined);
  const menuProps = useRowMenu(id, onMenu);
  // The secondary line and the details sit under the title, past the drag handle and the leading slot.
  const underTitle = (allowsDragging?: boolean) => `${(allowsDragging ? 1 : 0) + (leading != null ? 1 : 0) + 1} / -1`;
  return (
    <GridListItem
      {...rest}
      {...menuProps}
      id={id}
      textValue={text}
      isDisabled={disabled}
      data-current={current || undefined}
      // The row is a grid (its cell is display: contents): [drag handle] [leading] title [trailing] / secondary.
      style={({ allowsDragging }) => ({
        gridTemplateColumns: [allowsDragging && 'auto', leading != null && 'auto', 'minmax(0,1fr)', trailing != null && 'auto']
          .filter(Boolean)
          .join(' '),
      })}
      className={cn(
        'group/row relative grid cursor-default items-center gap-x-2 overflow-hidden rounded-md px-2 py-1 text-sm text-foreground outline-none select-none',
        'transition-colors duration-(--pdt-duration) motion-reduce:transition-none',
        'pointer-coarse:min-h-control-touch',
        'data-hovered:bg-muted data-selected:bg-accent data-current:bg-accent',
        'data-current:before:absolute data-current:before:inset-y-0 data-current:before:left-0 data-current:before:w-0.5 data-current:before:bg-primary',
        'data-focus-visible:ring-2 data-focus-visible:ring-ring data-focus-visible:ring-inset',
        'data-disabled:opacity-50 data-dragging:opacity-50 data-drop-target:bg-accent data-drop-target:ring-1 data-drop-target:ring-primary',
        className,
      )}
    >
      {({ allowsDragging }) => (
        <>
          {allowsDragging && (
            <AriaButton
              slot="drag"
              data-drag-handle
              className={cn(
                'flex size-4 items-center justify-center rounded-sm text-muted-foreground outline-none',
                'opacity-0 group-data-hovered/row:opacity-100 group-data-focus-visible/row:opacity-100 data-focus-visible:opacity-100 data-focus-visible:ring-2 data-focus-visible:ring-ring',
                'pointer-coarse:opacity-100',
              )}
            >
              <GripVertical size={12} aria-hidden />
            </AriaButton>
          )}
          {leading != null && <span className="flex items-center">{leading}</span>}
          <span className={cn('truncate', bold && 'font-semibold')}>{title}</span>
          {trailing != null && <span className="flex items-center gap-1.5">{trailing}</span>}
          {secondary != null && (
            <span
              data-slot="list-secondary"
              className="truncate text-meta text-muted-foreground"
              style={{ gridColumn: underTitle(allowsDragging) }}
            >
              {secondary}
            </span>
          )}
          {details != null && (
            <span
              data-slot="list-details"
              className="flex min-w-0 flex-col gap-0.5 text-xs text-muted-foreground"
              style={{ gridColumn: underTitle(allowsDragging) }}
            >
              {details}
            </span>
          )}
        </>
      )}
    </GridListItem>
  );
}
