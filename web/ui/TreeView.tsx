import { useRef, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { Button as AriaButton, Tree, TreeItem, TreeItemContent, type Key, type Selection } from 'react-aria-components';
import { cn } from './lib/utils';

/** One node of a tree. `hasChildren` marks a branch (a folder) whose children are loaded when it opens. */
export interface TreeNode {
  id: string;
  /** Shown, and used for typeahead and the accessible name. */
  label: string;
  hasChildren?: boolean;
  /** Before the label (a folder or file icon). */
  icon?: ReactNode;
  /** After the label, right-aligned (a git status letter). */
  trailing?: ReactNode;
  /** Extra classes for the label (e.g. `line-through` for a deleted file). */
  labelClassName?: string;
  /** Tooltip (e.g. the full path). */
  title?: string;
}

/** A branch's children as its loader sees them. */
export type TreeChildren<T extends TreeNode> =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; items: T[] };

export interface TreeViewProps<T extends TreeNode> {
  'aria-label': string;
  /** The top level. */
  items: T[];
  /**
   * A hook that loads an open branch's children, e.g. one `useQuery` per folder. It is called only while the branch is
   * expanded, from its own component, so it may use other hooks.
   */
  useChildren?: (node: T) => TreeChildren<T>;
  /** Expanded branch ids, controlled. Leave out (with `defaultExpandedKeys`) for uncontrolled. */
  expandedKeys?: Iterable<string>;
  defaultExpandedKeys?: Iterable<string>;
  onExpandedChange?: (keys: Set<string>) => void;
  /** The leaf shown as selected (controlled); leave out for uncontrolled. */
  selectedKey?: string | null;
  /** A leaf was opened: click, tap, Enter or Space. Branches toggle instead. */
  onAction?: (node: T) => void;
  /** Shown inside the tree when `items` is empty. */
  renderEmpty?: () => ReactNode;
  className?: string;
}

/** Placeholder rows (loading, error, empty, and the hidden child of a closed branch) get ids no path can have. */
type Placeholder = 'loading' | 'error' | 'empty' | 'closed';
const placeholderId = (parent: string, kind: Placeholder) => `${parent}\u0000${kind}`;

/**
 * A tree (React Aria Tree; ARIA tree keyboard: Up/Down, Left/Right to close/open, Home/End, typeahead) whose branches
 * load their children when opened. Single selection of leaves; branches open and close.
 */
export function TreeView<T extends TreeNode>({
  items,
  useChildren,
  expandedKeys,
  defaultExpandedKeys,
  onExpandedChange,
  selectedKey,
  onAction,
  renderEmpty,
  className,
  ...rest
}: TreeViewProps<T>) {
  const [ownExpanded, setOwnExpanded] = useState(() => new Set(defaultExpandedKeys ?? []));
  const expanded = expandedKeys !== undefined ? new Set(expandedKeys) : ownExpanded;
  const [ownSelected, setOwnSelected] = useState<string | null>(null);
  const selected = selectedKey !== undefined ? selectedKey : ownSelected;
  // Every node rendered so far, by id, so selection changes can hand the node back.
  const nodes = useRef(new Map<string, T>()).current;

  const setExpanded = (keys: Set<string>) => {
    if (expandedKeys === undefined) setOwnExpanded(keys);
    onExpandedChange?.(keys);
  };
  const toggle = (id: string) => {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpanded(next);
  };

  const renderNodes = (list: T[]): ReactNode =>
    list.map((node) => {
      nodes.set(node.id, node);
      const open = !!node.hasChildren && expanded.has(node.id);
      return (
        <TreeItem
          key={node.id}
          id={node.id}
          textValue={node.label}
          hasChildItems={!!node.hasChildren}
          // Branches can't be selected (disabledBehavior="selection"): pressing one toggles it.
          isDisabled={!!node.hasChildren}
          onAction={node.hasChildren ? () => toggle(node.id) : undefined}
          className={rowClass}
          style={indent}
        >
          <TreeItemContent>
            {({ hasChildItems, isExpanded }) => (
              <Row node={node} branch={hasChildItems} expanded={isExpanded} />
            )}
          </TreeItemContent>
          {/* React Aria only treats a row as a branch (aria-expanded, Right/Left) when it has child rows, so a closed
              branch gets one hidden placeholder child; its real children load when it opens. */}
          {node.hasChildren &&
            (open && useChildren ? (
              <Branch node={node} useChildren={useChildren} render={renderNodes} placeholder={placeholder} />
            ) : (
              placeholder(node.id, open ? 'empty' : 'closed', open ? 'Empty' : '')
            ))}
        </TreeItem>
      );
    }) as ReactNode;

  const placeholder = (parent: string, kind: Placeholder, text: string) => {
    const id = placeholderId(parent, kind);
    return (
      <TreeItem key={id} id={id} textValue={text} isDisabled className={rowClass} style={indent}>
        <TreeItemContent>
          {kind === 'loading' ? (
            <span role="img" aria-label={text} className="col-start-3 h-3 w-24 animate-pulse rounded-sm bg-muted motion-reduce:animate-none" />
          ) : kind === 'error' ? (
            <span role="alert" className="col-span-2 col-start-3 truncate text-xs text-destructive">{text}</span>
          ) : (
            <span className="col-start-3 text-xs text-muted-foreground">{text}</span>
          )}
        </TreeItemContent>
      </TreeItem>
    );
  };

  const content = renderNodes(items);
  return (
    <Tree
      {...rest}
      selectionMode="single"
      disallowEmptySelection={selected != null}
      selectedKeys={selected == null ? [] : [selected]}
      disabledBehavior="selection"
      onSelectionChange={(keys: Selection) => {
        if (keys === 'all') return;
        const key = [...keys][0];
        if (key == null) return;
        const node = nodes.get(String(key));
        if (!node || node.hasChildren) return;
        if (selectedKey === undefined) setOwnSelected(node.id);
        onAction?.(node);
      }}
      expandedKeys={expanded}
      onExpandedChange={(keys: Set<Key>) => setExpanded(new Set([...keys].map(String)))}
      renderEmptyState={renderEmpty ? () => renderEmpty() : undefined}
      className={cn('flex flex-col outline-none', className)}
    >
      {content}
    </Tree>
  );
}

/** An open branch's children: calls the loader hook (only while open) and renders what it returns. */
function Branch<T extends TreeNode>({ node, useChildren, render, placeholder }: {
  node: T;
  useChildren: (node: T) => TreeChildren<T>;
  render: (list: T[]) => ReactNode;
  placeholder: (parent: string, kind: Placeholder, text: string) => ReactNode;
}) {
  const children = useChildren(node);
  if (children.status === 'loading') return placeholder(node.id, 'loading', 'Loading…');
  if (children.status === 'error') return placeholder(node.id, 'error', children.message);
  if (children.items.length === 0) return placeholder(node.id, 'empty', 'Empty');
  return render(children.items);
}

const rowClass = cn(
  'group/row grid cursor-default grid-cols-[auto_auto_minmax(0,1fr)_auto] items-center gap-x-1 rounded-md py-0.5 pr-2 text-sm text-foreground outline-none select-none',
  'pointer-coarse:min-h-control-touch',
  'data-hovered:bg-muted data-selected:bg-accent',
  'data-focus-visible:ring-2 data-focus-visible:ring-ring data-focus-visible:ring-inset',
);

/** 12px per level, as FolderView does today. */
const indent = ({ level }: { level: number }) => ({ paddingInlineStart: `${(level - 1) * 12 + 4}px` });

function Row({ node, branch, expanded }: { node: TreeNode; branch: boolean; expanded: boolean }) {
  return (
    <>
      {branch ? (
        <AriaButton slot="chevron" excludeFromTabOrder className="flex size-3 items-center justify-center text-muted-foreground outline-none">
          <ChevronRight
            size={12}
            aria-hidden
            className={cn('transition-transform duration-(--pdt-duration) motion-reduce:transition-none', expanded && 'rotate-90')}
          />
        </AriaButton>
      ) : (
        <span className="w-3" />
      )}
      <span className="flex items-center text-muted-foreground">{node.icon}</span>
      <span className={cn('truncate', node.labelClassName)} title={node.title}>{node.label}</span>
      {node.trailing != null ? <span className="flex items-center">{node.trailing}</span> : <span />}
    </>
  );
}
