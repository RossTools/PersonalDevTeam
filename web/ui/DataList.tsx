import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ChevronDown, GripVertical } from 'lucide-react';
import {
  Button as AriaButton,
  Collection,
  DropIndicator,
  GridList,
  GridListHeader,
  GridListItem,
  GridListLoadMoreItem,
  GridListSection,
  ListLayout,
  Virtualizer,
  useDragAndDrop,
  type GridListItemProps,
  type Key,
  type Selection,
} from 'react-aria-components';
import { cn } from './lib/utils';
import { RowLongPress, RowMenus, openFocusedRowMenu, useRowMenu, type MenuPoint } from './lib/rowMenu';
import { Checkbox as CheckboxPrimitive } from './primitives/checkbox';
import { Spinner } from './Spinner';
import { dragData } from './List';

/** A group of rows under a header ("Ready · 12"). */
export interface DataListSection<T> {
  id: string;
  title: ReactNode;
  /** Shown after the title; defaults to the number of loaded items. */
  count?: number;
  /** Header only, rows hidden (the header's chevron calls `onToggleSection`). */
  collapsed?: boolean;
  items: T[];
}

/** A rank move: the dragged rows (`ids`, in list order; `id` is the first) go before or after `targetId`. */
export interface DataListMove {
  id: string;
  ids: string[];
  before?: string;
  after?: string;
}

export interface DataListProps<T extends { id: string }> {
  'aria-label': string;
  /** The rows (or use `sections`). Rendered lazily: only what is on screen mounts. */
  items?: T[];
  /** Grouped rows with headers and counts, instead of `items`. */
  sections?: DataListSection<T>[];
  /** Renders one row: return a `DataListRow` with `id={item.id}`. */
  children: (item: T) => ReactNode;
  /** Values the row renderer reads from outside the item (column visibility, now…); rows re-render when they change. */
  dependencies?: readonly unknown[];
  /** A fixed row height in px (fastest; at least 44 on touch screens). Without it rows are measured (estimate 36px). */
  rowHeight?: number;
  /** Alternate rows get a faint fill (counted from the top of each group) so long lists are easier to read across. */
  striped?: boolean;
  /** Multi-select with a checkbox per row: Shift-click for ranges, Cmd/Ctrl+A for all, Escape to clear. */
  selectable?: boolean;
  /** Controlled selection (ids). Leave out for the list to keep it. */
  selectedKeys?: ReadonlySet<string>;
  onSelectionChange?: (keys: Set<string>) => void;
  /** Click, tap or Enter on a row (while rows are selected, a click toggles selection instead). */
  onAction?: (id: string) => void;
  /** Rows get a drag handle and carry text/plain `pdt:<dragKind>:<id>` (so other targets, e.g. a board, can take them). */
  dragKind?: string;
  /** A drop between rows of this list (needs `dragKind`). */
  onMove?: (move: DataListMove) => void;
  /** Alt+↑ / Alt+↓ on the focused row. */
  onMoveUp?: (id: string) => void;
  onMoveDown?: (id: string) => void;
  /** Section headers get a collapse chevron. */
  onToggleSection?: (id: string) => void;
  /** Called when the user scrolls near the end, to load the next page. */
  onEndReached?: () => void;
  /** A page is loading: shows a spinner row at the end. */
  loadingMore?: boolean;
  /** Shown inside the list when it has no rows (an `EmptyState`). */
  renderEmpty?: () => ReactNode;
  /** Give the list a height (it is its own scroll container), e.g. `flex-1 min-h-0`. */
  className?: string;
}

const EMPTY: ReadonlySet<string> = new Set();
/** Rows are at least 44px on touch screens (`pointer-coarse:min-h-control-touch`); fixed heights follow. */
const LONG_PRESS_SLACK_MS = 400;
/** The ids of rows that get the stripe fill (null: the list is not striped). */
const Banded = createContext<ReadonlySet<string> | null>(null);
const TOUCH_ROW = 44;

function useCoarsePointer(): boolean {
  const [coarse, setCoarse] = useState(() => typeof window !== 'undefined' && window.matchMedia?.('(pointer: coarse)').matches === true);
  useEffect(() => {
    const mq = window.matchMedia?.('(pointer: coarse)');
    if (!mq) return;
    const on = () => setCoarse(mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);
  return coarse;
}

/**
 * A virtualised list for large collections (React Aria GridList + Virtualizer): only the rows on screen mount, so
 * thousands of rows scroll smoothly. Optional checkboxes, rank drag with a handle, Alt+↑/↓ moves, group headers and
 * infinite paging. Rows are `DataListRow`s.
 */
export function DataList<T extends { id: string }>({
  items,
  sections,
  children,
  dependencies,
  rowHeight,
  striped = false,
  selectable = false,
  selectedKeys,
  onSelectionChange,
  onAction,
  dragKind,
  onMove,
  onMoveUp,
  onMoveDown,
  onToggleSection,
  onEndReached,
  loadingMore = false,
  renderEmpty,
  className,
  'aria-label': ariaLabel,
}: DataListProps<T>) {
  const menus = useRef(new Map<string, (p: MenuPoint) => void>()).current;
  const coarse = useCoarsePointer();
  const fixedHeight = rowHeight && coarse ? Math.max(rowHeight, TOUCH_ROW) : rowHeight;
  const [ownSelection, setOwnSelection] = useState<ReadonlySet<string>>(EMPTY);
  const selected = selectedKeys ?? ownSelection;
  const setSelected = (next: Set<string>) => {
    if (selectedKeys === undefined) setOwnSelection(next);
    onSelectionChange?.(next);
  };
  // The ids in display order (collapsed sections hidden), for Shift ranges and ordering multi-row drags.
  const order = useMemo(
    () => (sections ? sections.flatMap((s) => (s.collapsed ? [] : s.items)) : (items ?? [])).map((i) => i.id),
    [items, sections],
  );
  // Odd rows of each group (or of the whole list), for `striped`.
  const banded = useMemo(() => {
    if (!striped) return null;
    const groups = sections ? sections.map((s) => s.items) : [items ?? []];
    return new Set(groups.flatMap((g) => g.filter((_, i) => i % 2 === 1).map((r) => r.id)));
  }, [striped, items, sections]);
  const shift = useRef(false);
  const anchor = useRef<string | null>(null);

  const range = (from: string, to: string): string[] => {
    const a = order.indexOf(from);
    const b = order.indexOf(to);
    if (a < 0 || b < 0) return [to];
    return order.slice(Math.min(a, b), Math.max(a, b) + 1);
  };

  // A touch long press opens the row's menu; React Aria also toggles the row's selection on it, within a few ms
  // either side of our timer. Whichever comes second undoes the toggle.
  const longPress = useRef<{ id: string; at: number } | null>(null);
  const lastToggle = useRef<{ id: string; at: number; before: ReadonlySet<string> } | null>(null);
  const onLongPress = (id: string) => {
    const t = lastToggle.current;
    if (t && t.id === id && Date.now() - t.at < LONG_PRESS_SLACK_MS) {
      lastToggle.current = null;
      setSelected(new Set(t.before));
      return;
    }
    longPress.current = { id, at: Date.now() };
  };
  const onRacSelection = (keys: Selection) => {
    const next = keys === 'all' ? new Set(order) : new Set([...keys].map(String));
    const added = [...next].filter((k) => !selected.has(k));
    const removed = [...selected].filter((k) => !next.has(k));
    if (added.length + removed.length === 1) {
      const id = added[0] ?? removed[0];
      const lp = longPress.current;
      if (lp && lp.id === id && Date.now() - lp.at < LONG_PRESS_SLACK_MS) {
        longPress.current = null;
        return;
      }
      lastToggle.current = { id, at: Date.now(), before: selected };
    }
    // React Aria's checkbox only toggles; a Shift-click on one extends from the last toggled row.
    if (shift.current && added.length === 1 && removed.length === 0 && anchor.current && anchor.current !== added[0]) {
      for (const k of range(anchor.current, added[0])) next.add(k);
    } else if (added.length + removed.length === 1) {
      anchor.current = added[0] ?? removed[0];
    }
    setSelected(next);
  };

  const { dragAndDropHooks } = useDragAndDrop<T>({
    getItems: (keys) =>
      [...keys].map((k) => ({ 'text/plain': dragData(dragKind ?? 'item', String(k)) })),
    getAllowedDropOperations: () => ['move'],
    onReorder: onMove
      ? (e) => {
          const ids = [...e.keys].map(String).sort((a, b) => order.indexOf(a) - order.indexOf(b));
          const target = String(e.target.key);
          if (ids.includes(target)) return;
          onMove({ id: ids[0], ids, ...(e.target.dropPosition === 'after' ? { after: target } : { before: target }) });
        }
      : undefined,
    renderDropIndicator: (target) => (
      <DropIndicator target={target} className="mx-2 h-0 outline-1 outline-primary data-drop-target:outline-2" />
    ),
  });

  const renderSection = (s: DataListSection<T>) => (
    <GridListSection id={`section:${s.id}`}>
      <GridListHeader className="flex h-8 pointer-coarse:h-control-touch items-center gap-1.5 border-b border-border bg-background px-2 text-xs font-medium text-muted-foreground">
        {onToggleSection && (
          <button
            type="button"
            aria-expanded={!s.collapsed}
            aria-label={s.collapsed ? 'Expand group' : 'Collapse group'}
            onClick={() => onToggleSection(s.id)}
            className="-ml-1 flex size-5 items-center justify-center rounded-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:size-control-touch"
          >
            <ChevronDown aria-hidden className={cn('size-3.5 transition-transform motion-reduce:transition-none', s.collapsed && '-rotate-90')} />
          </button>
        )}
        <span className="truncate text-foreground">{s.title}</span>
        <span className="tabular-nums">{s.count ?? s.items.length}</span>
      </GridListHeader>
      <Collection items={s.collapsed ? [] : s.items} dependencies={dependencies ? [...dependencies] : undefined}>
        {children}
      </Collection>
    </GridListSection>
  );

  return (
    <RowMenus.Provider value={menus}>
      <RowLongPress.Provider value={onLongPress}>
        <Banded.Provider value={banded}>
          <div
            className="contents"
            onPointerDownCapture={(e) => { shift.current = e.shiftKey; }}
            onKeyDownCapture={(e) => {
              shift.current = false;
              if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
              const handler = e.key === 'ArrowUp' ? onMoveUp : onMoveDown;
              const row = (e.target as HTMLElement).closest<HTMLElement>('[role="row"][data-key]');
              if (!handler || !row?.dataset.key) return;
              e.preventDefault();
              e.stopPropagation();
              handler(row.dataset.key);
            }}
            onKeyDown={(e) => openFocusedRowMenu(e, menus)}
          >
            <Virtualizer
              layout={ListLayout}
              layoutOptions={
                fixedHeight
                  ? { rowHeight: fixedHeight, headingHeight: coarse ? TOUCH_ROW : 32, loaderHeight: 32, dropIndicatorThickness: 2 }
                  : { estimatedRowHeight: coarse ? TOUCH_ROW : 36, estimatedHeadingHeight: coarse ? TOUCH_ROW : 32, loaderHeight: 32, dropIndicatorThickness: 2 }
              }
            >
              <GridList
                aria-label={ariaLabel}
                layout="stack"
                selectionMode={selectable ? 'multiple' : 'none'}
                selectionBehavior="toggle"
                selectedKeys={selectable ? selected : undefined}
                onSelectionChange={selectable ? onRacSelection : undefined}
                onAction={
                  onAction || selectable
                    ? (key: Key) => {
                        const id = String(key);
                        // Shift-click with nothing selected starts a selection rather than opening the row.
                        if (selectable && shift.current) {
                          const next = new Set(selected);
                          for (const k of anchor.current ? range(anchor.current, id) : [id]) next.add(k);
                          anchor.current ??= id;
                          setSelected(next);
                          return;
                        }
                        onAction?.(id);
                      }
                    : undefined
                }
                renderEmptyState={renderEmpty ? () => renderEmpty() : undefined}
                dragAndDropHooks={dragKind ? dragAndDropHooks : undefined}
                className={cn('relative overflow-auto outline-none', className)}
              >
                {sections ? (
                  <Collection items={sections} dependencies={dependencies ? [...dependencies] : undefined}>
                    {renderSection}
                  </Collection>
                ) : (
                  <Collection items={items ?? []} dependencies={dependencies ? [...dependencies] : undefined}>
                    {children}
                  </Collection>
                )}
                {onEndReached && (
                  <GridListLoadMoreItem onLoadMore={() => { if (!loadingMore) onEndReached(); }} isLoading={loadingMore} className="flex h-8 items-center justify-center">
                    <Spinner />
                  </GridListLoadMoreItem>
                )}
              </GridList>
            </Virtualizer>
          </div>
        </Banded.Provider>
      </RowLongPress.Provider>
    </RowMenus.Provider>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// DataListRow

export interface DataListRowProps
  extends Omit<GridListItemProps, 'children' | 'className' | 'id' | 'textValue' | 'isDisabled' | 'style'> {
  id: string;
  /** Text for typeahead and the row's accessible name. */
  textValue: string;
  /** The row's content (key, title, badges…), laid out by the caller. */
  children: ReactNode;
  /** Before the content (after the handle and checkbox). */
  leading?: ReactNode;
  /** Pinned to the end: updated time, a menu button. */
  trailing?: ReactNode;
  /** Unseen: semibold content. */
  bold?: boolean;
  disabled?: boolean;
  /** Opens the row's menu: right-click, long press on touch, Shift+F10 or the Menu key. */
  onMenu?: (at: MenuPoint) => void;
  className?: string;
}

/** One `DataList` row: [drag handle] [checkbox] [leading] content [trailing]. */
export function DataListRow({ id, textValue, children, leading, trailing, bold = false, disabled, onMenu, className, ...rest }: DataListRowProps) {
  const menuProps = useRowMenu(id, onMenu);
  const striped = useContext(Banded)?.has(id) ?? false;
  return (
    <GridListItem
      {...rest}
      {...menuProps}
      id={id}
      textValue={textValue}
      isDisabled={disabled}
      className={cn(
        'group/row flex min-h-9 cursor-default items-center gap-2 border-b border-border px-2 py-1 text-sm text-foreground outline-none select-none',
        striped && 'bg-band',
        'pointer-coarse:min-h-control-touch',
        'data-hovered:bg-muted data-selected:bg-accent',
        'data-focus-visible:ring-2 data-focus-visible:ring-ring data-focus-visible:ring-inset',
        'data-disabled:opacity-50 data-dragging:opacity-50 data-drop-target:bg-accent',
        className,
      )}
    >
      {({ allowsDragging, selectionMode }) => (
        <>
          {allowsDragging && (
            <AriaButton
              slot="drag"
              data-drag-handle
              className={cn(
                'flex size-4 shrink-0 items-center justify-center rounded-sm text-muted-foreground outline-none',
                'opacity-0 group-data-hovered/row:opacity-100 group-data-focus-visible/row:opacity-100 data-focus-visible:opacity-100 data-focus-visible:ring-2 data-focus-visible:ring-ring',
                'pointer-coarse:opacity-100',
              )}
            >
              <GripVertical size={12} aria-hidden />
            </AriaButton>
          )}
          {selectionMode === 'multiple' && <CheckboxPrimitive slot="selection" className="shrink-0" />}
          {leading != null && <span className="flex shrink-0 items-center">{leading}</span>}
          <div className={cn('flex min-w-0 flex-1 items-center gap-2', bold && 'font-semibold')}>{children}</div>
          {trailing != null && <span className="flex shrink-0 items-center gap-1.5">{trailing}</span>}
        </>
      )}
    </GridListItem>
  );
}
