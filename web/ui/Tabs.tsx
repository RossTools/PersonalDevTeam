import type { ReactNode, Ref, UIEventHandler } from 'react';
import { Tab, TabList, TabPanel as TabPanelPrimitive, Tabs as TabsPrimitive, type Key } from 'react-aria-components';
import { cn } from '@ui/lib/utils';

export interface TabItem<K extends string = string> {
  id: K;
  /** The tab's text, optionally with something after it (a count, a diff size). */
  label: ReactNode;
  disabled?: boolean;
}

export interface TabsProps<K extends string = string> {
  tabs: TabItem<K>[];
  value: K;
  onChange: (id: K) => void;
  /** Names the tab list ("Ticket sections"). */
  'aria-label': string;
  /** The `TabPanel`s, one per tab (only the selected one renders). Put each straight inside: whatever sits between
   * `Tabs` and a `TabPanel` renders twice, once to collect the tabs. */
  children: ReactNode;
  /** On the wrapper around the list and the panels: a flex column by default. */
  className?: string;
  /** On the tab list (its padding, to line up with the page). */
  listClassName?: string;
  /** At the end of the tab row, right-aligned: a status that holds whichever tab is shown (it brings its own right
   * padding, to line up with the page like `listClassName`). */
  end?: ReactNode;
}

/**
 * A page's sections as underlined tabs (Overview · Conversation · Changes). Left/Right/Home/End move between the tabs
 * and show the one they land on; the list is one Tab stop. Each tab's content is a `TabPanel` with the tab's `id`.
 * Editor-style tabs that are dragged, renamed or closed (a workspace's tabs) stay hand-built (`tabListKeyDown`).
 */
export function Tabs<K extends string = string>({ tabs, value, onChange, children, className, listClassName, end, ...aria }: TabsProps<K>) {
  const list = (
    <TabList
      {...aria}
      className={cn('flex shrink-0 items-center gap-1 px-3', !end && 'border-b border-border', listClassName)}
    >
      {tabs.map((t) => (
        <Tab
          key={t.id}
          id={t.id}
          className={cn(
            '-mb-px flex shrink-0 cursor-default items-center border-b-2 border-transparent px-2 py-1.5 text-xs whitespace-nowrap text-muted-foreground outline-none',
            'hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:min-h-control-touch',
            'data-selected:border-primary data-selected:font-medium data-selected:text-foreground data-disabled:opacity-50',
          )}
        >
          {t.label}
        </Tab>
      ))}
    </TabList>
  );
  return (
    <TabsPrimitive
      selectedKey={value}
      onSelectionChange={(k: Key) => { if (k !== value) onChange(String(k) as K); }}
      disabledKeys={tabs.filter((t) => t.disabled).map((t) => t.id)}
      className={cn('flex min-h-0 flex-1 flex-col', className)}
    >
      {!end ? list : (
        // The row's rule is under both, so the selected tab's underline still sits on it.
        <div className="flex shrink-0 border-b border-border">
          {list}
          <div className="flex min-w-0 flex-1 items-center justify-end pl-2">{end}</div>
        </div>
      )}
      {children}
    </TabsPrimitive>
  );
}

/**
 * What a tab shows: inside `Tabs`, with the tab's `id`. A flex column that fills the space under the tabs by default;
 * give it `overflow-auto` (and `ref`/`onScroll`, to remember where it was scrolled) when the panel itself scrolls.
 */
export function TabPanel({ id, children, className, ref, onScroll }: {
  id: string;
  children: ReactNode;
  className?: string;
  ref?: Ref<HTMLDivElement>;
  onScroll?: UIEventHandler<HTMLDivElement>;
}) {
  return (
    <TabPanelPrimitive id={id} ref={ref} onScroll={onScroll} className={cn('flex min-h-0 flex-1 flex-col outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset', className)}>
      {children}
    </TabPanelPrimitive>
  );
}
