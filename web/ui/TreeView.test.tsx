import { useState } from 'react';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { TreeView, type TreeChildren, type TreeNode } from './TreeView';

const root: TreeNode[] = [
  { id: 'src', label: 'src', hasChildren: true },
  { id: 'README.md', label: 'README.md', trailing: <span>M</span> },
];
const tree: Record<string, TreeNode[]> = {
  src: [
    { id: 'src/lib', label: 'lib', hasChildren: true },
    { id: 'src/main.ts', label: 'main.ts' },
  ],
  'src/lib': [{ id: 'src/lib/a.ts', label: 'a.ts' }],
};

/** A loader like FolderView's: one "fetch" per opened folder, resolved by the test. */
function useFakeChildren(loaded: Set<string>, failed?: string) {
  return (node: TreeNode): TreeChildren<TreeNode> =>
    failed === node.id
      ? { status: 'error', message: 'Permission denied' }
      : loaded.has(node.id)
        ? { status: 'ready', items: tree[node.id] ?? [] }
        : { status: 'loading' };
}

function Files(props: { loaded?: Set<string>; failed?: string; onAction?: (n: TreeNode) => void; calls?: string[] }) {
  const loaded = props.loaded ?? new Set(Object.keys(tree));
  const load = useFakeChildren(loaded, props.failed);
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <TreeView
      aria-label="Files"
      items={root}
      useChildren={(n) => {
        props.calls?.push(n.id);
        return load(n);
      }}
      selectedKey={selected}
      onAction={(n) => {
        setSelected(n.id);
        props.onAction?.(n);
      }}
    />
  );
}

describe('TreeView', () => {
  it('renders a labelled tree with branches collapsed and the trailing slot', () => {
    render(<Files />);
    const t = screen.getByRole('treegrid', { name: 'Files' });
    const src = within(t).getByRole('row', { name: 'src' });
    expect(src).toHaveAttribute('aria-expanded', 'false');
    expect(within(t).getByRole('row', { name: /README\.md/ })).toHaveTextContent('M');
    expect(within(t).queryByRole('row', { name: 'main.ts' })).toBeNull();
  });

  it('loads children only for open branches', async () => {
    const calls: string[] = [];
    render(<Files calls={calls} />);
    expect(calls).toEqual([]);
    await userEvent.click(screen.getByRole('row', { name: 'src' }));
    expect(screen.getByRole('row', { name: 'src' })).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('row', { name: 'main.ts' })).toBeInTheDocument();
    expect(new Set(calls)).toEqual(new Set(['src']));
  });

  it('shows a loading row, then an error row, for a branch', async () => {
    const { rerender } = render(<Files loaded={new Set()} />);
    await userEvent.click(screen.getByRole('row', { name: 'src' }));
    expect(screen.getByRole('img', { name: 'Loading…' })).toBeInTheDocument();
    rerender(<Files loaded={new Set()} failed="src" />);
    expect(screen.getByRole('alert')).toHaveTextContent('Permission denied');
  });

  it('opening a leaf calls onAction and marks it selected; branches are never selected', async () => {
    const onAction = vi.fn();
    render(<Files onAction={onAction} />);
    await userEvent.click(screen.getByRole('row', { name: /README\.md/ }));
    expect(onAction).toHaveBeenCalledWith(expect.objectContaining({ id: 'README.md' }));
    expect(screen.getByRole('row', { name: /README\.md/ })).toHaveAttribute('aria-selected', 'true');
    // A branch click with a leaf selected still toggles the branch (and leaves the selection alone).
    await userEvent.click(screen.getByRole('row', { name: 'src' }));
    expect(screen.getByRole('row', { name: 'src' })).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('row', { name: /README\.md/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('row', { name: 'main.ts' }));
    expect(onAction).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'src/main.ts' }));
  });

  it('keyboard: arrows move, Right opens, Left closes, Enter opens a leaf', async () => {
    const onAction = vi.fn();
    render(<Files onAction={onAction} />);
    await userEvent.tab();
    const src = screen.getByRole('row', { name: 'src' });
    expect(src).toHaveFocus();
    await userEvent.keyboard('{ArrowRight}');
    expect(src).toHaveAttribute('aria-expanded', 'true');
    await userEvent.keyboard('{ArrowDown}{ArrowDown}');
    expect(screen.getByRole('row', { name: 'main.ts' })).toHaveFocus();
    await userEvent.keyboard('{Enter}');
    expect(onAction).toHaveBeenCalledWith(expect.objectContaining({ id: 'src/main.ts' }));
    await userEvent.keyboard('{ArrowUp}{ArrowUp}{ArrowLeft}');
    expect(src).toHaveAttribute('aria-expanded', 'false');
    await userEvent.keyboard('{End}');
    expect(screen.getByRole('row', { name: /README\.md/ })).toHaveFocus();
  });

  it('expanded state can be controlled', async () => {
    const onExpandedChange = vi.fn();
    render(
      <TreeView aria-label="Files" items={root} expandedKeys={['src']} onExpandedChange={onExpandedChange}
        useChildren={(n) => ({ status: 'ready', items: tree[n.id] ?? [] })} />,
    );
    expect(screen.getByRole('row', { name: 'lib' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('row', { name: 'src' }));
    expect(onExpandedChange).toHaveBeenCalledWith(new Set());
    // Controlled: still open until the parent says otherwise.
    expect(screen.getByRole('row', { name: 'lib' })).toBeInTheDocument();
  });

  it('an open empty folder says so and can still be closed', async () => {
    render(<TreeView aria-label="Files" items={[{ id: 'e', label: 'e', hasChildren: true }]} defaultExpandedKeys={['e']}
      useChildren={() => ({ status: 'ready', items: [] })} />);
    expect(screen.getByRole('row', { name: 'Empty' })).toBeInTheDocument();
    await userEvent.tab();
    await userEvent.keyboard('{ArrowLeft}');
    expect(screen.getByRole('row', { name: 'e' })).toHaveAttribute('aria-expanded', 'false');
  });

  it('shows the empty state', () => {
    render(<TreeView aria-label="Files" items={[]} renderEmpty={() => 'No files.'} />);
    expect(screen.getByRole('treegrid', { name: 'Files' })).toHaveTextContent('No files.');
  });
});
