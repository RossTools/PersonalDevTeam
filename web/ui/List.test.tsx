import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { List, ListRow, dragData, parseDragData, type ListDropEvent, type ListProps } from './List';

const rows = [
  { id: 'a', title: 'Alpha' },
  { id: 'b', title: 'Beta' },
  { id: 'c', title: 'Gamma' },
];

type Row = (typeof rows)[number];

function Three(props: Omit<ListProps<Row>, 'aria-label' | 'items' | 'children'>) {
  return (
    <List aria-label="Agents" items={rows} {...props}>
      {(r) => <ListRow id={r.id} title={r.title} current={r.id === 'b'} />}
    </List>
  );
}

afterEach(() => vi.useRealTimers());

describe('List and ListRow', () => {
  it('renders a labelled grid of rows with title, secondary, leading and trailing', () => {
    render(
      <List aria-label="Agents">
        <ListRow id="a" title="fix login" secondary="Workspace PDT · main" leading={<span data-testid="dot" />}
          trailing={<span>Working</span>} bold />
      </List>,
    );
    const list = screen.getByRole('grid', { name: 'Agents' });
    const row = within(list).getByRole('row', { name: /fix login/ });
    expect(row).toHaveTextContent('Workspace PDT · main');
    expect(within(row).getByTestId('dot')).toBeInTheDocument();
    expect(within(row).getByText('Working')).toBeInTheDocument();
    expect(within(row).getByText('fix login').className).toContain('font-semibold');
  });

  it('marks the current row with the accent fill', () => {
    render(<Three />);
    const row = screen.getByRole('row', { name: 'Beta' });
    expect(row).toHaveAttribute('data-current');
    expect(row.className).toContain('data-current:bg-accent');
    expect(screen.getByRole('row', { name: 'Alpha' })).not.toHaveAttribute('data-current');
  });

  it('fires onAction on click and on Enter', async () => {
    const onAction = vi.fn();
    render(<Three onAction={onAction} />);
    await userEvent.click(screen.getByRole('row', { name: 'Gamma' }));
    expect(onAction).toHaveBeenLastCalledWith('c');
    await userEvent.keyboard('{ArrowUp}{Enter}');
    expect(onAction).toHaveBeenLastCalledWith('b');
  });

  it('moves with arrow keys, Home and End', async () => {
    render(<Three onAction={() => {}} />);
    await userEvent.tab();
    expect(screen.getByRole('row', { name: 'Alpha' })).toHaveFocus();
    await userEvent.keyboard('{ArrowDown}');
    expect(screen.getByRole('row', { name: 'Beta' })).toHaveFocus();
    await userEvent.keyboard('{End}');
    expect(screen.getByRole('row', { name: 'Gamma' })).toHaveFocus();
    await userEvent.keyboard('{Home}');
    expect(screen.getByRole('row', { name: 'Alpha' })).toHaveFocus();
  });

  it('single selection reports the selected id', async () => {
    const onSelectionChange = vi.fn();
    render(<Three selectionMode="single" onSelectionChange={onSelectionChange} />);
    await userEvent.click(screen.getByRole('row', { name: 'Gamma' }));
    expect(onSelectionChange).toHaveBeenLastCalledWith('c');
    expect(screen.getByRole('row', { name: 'Gamma' })).toHaveAttribute('aria-selected', 'true');
  });

  it('shows the empty state inside the list', () => {
    render(<List aria-label="Agents" items={[]} renderEmpty={() => <p>No agents yet.</p>}>{() => null}</List>);
    expect(within(screen.getByRole('grid', { name: 'Agents' })).getByText('No agents yet.')).toBeInTheDocument();
  });

  it('opens the row menu on right-click with the pointer position', () => {
    const onMenu = vi.fn();
    render(<List aria-label="Agents"><ListRow id="a" title="Alpha" onMenu={onMenu} /></List>);
    fireEvent.contextMenu(screen.getByRole('row', { name: 'Alpha' }), { clientX: 30, clientY: 40 });
    expect(onMenu).toHaveBeenCalledWith({ x: 30, y: 40 });
  });

  it('opens the row menu on a long press (touch), once', () => {
    vi.useFakeTimers();
    const onMenu = vi.fn();
    render(<List aria-label="Agents"><ListRow id="a" title="Alpha" onMenu={onMenu} /></List>);
    const row = screen.getByRole('row', { name: 'Alpha' });
    fireEvent.pointerDown(row, { pointerType: 'touch', clientX: 5, clientY: 6 });
    act(() => vi.advanceTimersByTime(600));
    fireEvent.pointerUp(row, { pointerType: 'touch' });
    // Android follows a long press with a contextmenu event: it must not open the menu twice.
    fireEvent.contextMenu(row);
    expect(onMenu).toHaveBeenCalledTimes(1);
    expect(onMenu).toHaveBeenCalledWith({ x: 5, y: 6 });
  });

  it('a long press that opened the menu does not also act on the row (iOS sends no click after it)', () => {
    vi.useFakeTimers();
    const onAction = vi.fn();
    render(<List aria-label="Agents" onAction={onAction}><ListRow id="a" title="Alpha" onMenu={() => {}} /></List>);
    const row = screen.getByRole('row', { name: 'Alpha' });
    const touch = { pointerType: 'touch', pointerId: 1, button: 0, width: 1, height: 1, clientX: 5, clientY: 6 };
    fireEvent.pointerDown(row, touch);
    act(() => vi.advanceTimersByTime(600));
    fireEvent.pointerUp(row, touch);
    act(() => vi.advanceTimersByTime(1000));
    expect(onAction).not.toHaveBeenCalled();
  });

  it('a short tap does not open the menu', () => {
    vi.useFakeTimers();
    const onMenu = vi.fn();
    render(<List aria-label="Agents"><ListRow id="a" title="Alpha" onMenu={onMenu} /></List>);
    const row = screen.getByRole('row', { name: 'Alpha' });
    fireEvent.pointerDown(row, { pointerType: 'touch' });
    act(() => vi.advanceTimersByTime(200));
    fireEvent.pointerUp(row, { pointerType: 'touch' });
    act(() => vi.advanceTimersByTime(600));
    expect(onMenu).not.toHaveBeenCalled();
  });

  it('opens the focused row menu with Shift+F10', async () => {
    const onMenu = vi.fn();
    render(
      <List aria-label="Agents">
        <ListRow id="a" title="Alpha" />
        <ListRow id="b" title="Beta" onMenu={onMenu} />
      </List>,
    );
    await userEvent.tab();
    await userEvent.keyboard('{ArrowDown}{Shift>}{F10}{/Shift}');
    expect(onMenu).toHaveBeenCalledTimes(1);
  });

  it('passes DOM props through (onContextMenu from an outside menu helper)', () => {
    const onContextMenu = vi.fn();
    render(<List aria-label="Agents"><ListRow id="a" title="Alpha" onContextMenu={onContextMenu} /></List>);
    fireEvent.contextMenu(screen.getByRole('row', { name: 'Alpha' }));
    expect(onContextMenu).toHaveBeenCalled();
  });
});

describe('drag and drop', () => {
  it('drag data is text/plain pdt:<kind>:<id>', () => {
    expect(dragData('tab', 't1')).toBe('pdt:tab:t1');
    expect(parseDragData('pdt:workspace:w:1')).toEqual({ kind: 'workspace', id: 'w:1' });
    expect(parseDragData('hello')).toBeNull();
    expect(parseDragData('pdt:tab:')).toBeNull();
  });

  it('draggable rows get a drag handle; plain rows do not', () => {
    const { unmount } = render(<Three dragKind="workspace" />);
    const handle = within(screen.getByRole('row', { name: /Alpha/ })).getByRole('button');
    expect(handle).toHaveAttribute('aria-label', expect.stringMatching(/drag/i));
    unmount();
    render(<Three />);
    expect(within(screen.getByRole('row', { name: 'Alpha' })).queryByRole('button')).toBeNull();
  });

  it('moves a row by keyboard drag and reports pdt:<kind>:<id> with the target', async () => {
    const onDrop = vi.fn<(e: ListDropEvent) => void>();
    render(<Three dragKind="workspace" onDrop={onDrop} />);
    const handle = within(screen.getByRole('row', { name: /Alpha/ })).getByRole('button');
    act(() => handle.focus());
    await userEvent.keyboard('{Enter}');
    // React Aria moves focus to the first drop target once the drag starts (asynchronously).
    const focused = () => document.activeElement?.getAttribute('aria-label') ?? '';
    await vi.waitFor(() => expect(focused()).toMatch(/^Insert/));
    for (let i = 0; i < 8 && focused() !== 'Insert between Beta and Gamma'; i++) await userEvent.keyboard('{ArrowDown}');
    expect(focused()).toBe('Insert between Beta and Gamma');
    await userEvent.keyboard('{Enter}');
    await vi.waitFor(() => expect(onDrop).toHaveBeenCalledTimes(1));
    const e = onDrop.mock.calls[0][0];
    expect(e).toMatchObject({ kind: 'workspace', id: 'a' });
    expect([{ id: 'b', position: 'after' }, { id: 'c', position: 'before' }]).toContainEqual(e.target);
  });
});
