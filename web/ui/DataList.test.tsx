import { useState } from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { DataList, DataListRow, type DataListProps, type DataListSection } from './DataList';

// jsdom has no layout: give every element a viewport so the virtualizer renders the first screen of rows.
const sizes: [keyof HTMLElement, number][] = [['clientWidth', 400], ['clientHeight', 360], ['offsetWidth', 400], ['offsetHeight', 360]];
const restore: (() => void)[] = [];
let observed: ((entries: { isIntersecting: boolean }[]) => void)[] = [];
beforeAll(() => {
  for (const [prop, value] of sizes) {
    const spy = vi.spyOn(HTMLElement.prototype, prop as 'clientWidth', 'get').mockImplementation(() => value);
    restore.push(() => spy.mockRestore());
  }
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(cb: (entries: { isIntersecting: boolean }[]) => void) { observed.push(cb); }
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});
afterAll(() => {
  restore.forEach((r) => r());
  vi.unstubAllGlobals();
});

interface Row { id: string; title: string }
const many = (n: number): Row[] => Array.from({ length: n }, (_, i) => ({ id: `t${i}`, title: `Ticket ${i}` }));

type Props = Omit<DataListProps<Row>, 'aria-label' | 'children'>;

function Rows(props: Props) {
  return (
    <DataList aria-label="Tickets" rowHeight={36} {...props}>
      {(r) => (
        <DataListRow id={r.id} textValue={r.title}>
          <span>{r.title}</span>
        </DataListRow>
      )}
    </DataList>
  );
}

function Selectable(props: Props & { onChange?: (keys: Set<string>) => void }) {
  const [keys, setKeys] = useState<Set<string>>(new Set());
  return (
    <Rows
      {...props}
      selectable
      selectedKeys={keys}
      onSelectionChange={(k) => { setKeys(k); props.onChange?.(k); }}
    />
  );
}

const row = (name: string) => screen.getByRole('row', { name });

describe('DataList', () => {
  it('mounts only the rows on screen out of thousands', () => {
    render(<Rows items={many(2000)} />);
    const grid = screen.getByRole('grid', { name: 'Tickets' });
    const rows = within(grid).getAllByRole('row');
    expect(rows.length).toBeGreaterThan(5);
    expect(rows.length).toBeLessThan(60);
    expect(row('Ticket 0')).toBeInTheDocument();
    expect(screen.queryByRole('row', { name: 'Ticket 1999' })).toBeNull();
  });

  it('opens a row on click and Enter', async () => {
    const onAction = vi.fn();
    render(<Rows items={many(5)} onAction={onAction} />);
    await userEvent.click(row('Ticket 2'));
    expect(onAction).toHaveBeenLastCalledWith('t2');
    await userEvent.keyboard('{ArrowDown}{Enter}');
    expect(onAction).toHaveBeenLastCalledWith('t3');
  });

  it('selects with checkboxes, Shift-click ranges, Cmd/Ctrl+A and Escape', async () => {
    const onChange = vi.fn();
    const onAction = vi.fn();
    render(<Selectable items={many(10)} onChange={onChange} onAction={onAction} />);
    const box = (name: string) => within(row(name)).getByRole('checkbox');
    await userEvent.click(box('Ticket 1'));
    expect([...onChange.mock.lastCall![0]]).toEqual(['t1']);
    expect(row('Ticket 1')).toHaveAttribute('aria-selected', 'true');

    const user = userEvent.setup();
    await user.keyboard('{Shift>}');
    await user.click(box('Ticket 4'));
    await user.keyboard('{/Shift}');
    expect([...onChange.mock.lastCall![0]].sort()).toEqual(['t1', 't2', 't3', 't4']);

    await user.keyboard('{Control>}a{/Control}');
    expect(onChange.mock.lastCall![0].size).toBe(10);
    await user.keyboard('{Escape}');
    expect(onChange.mock.lastCall![0].size).toBe(0);
    expect(onAction).not.toHaveBeenCalled();
  });

  it('Shift-click on a row with nothing selected starts a selection instead of opening it', async () => {
    const onChange = vi.fn();
    const onAction = vi.fn();
    render(<Selectable items={many(5)} onChange={onChange} onAction={onAction} />);
    const user = userEvent.setup();
    await user.keyboard('{Shift>}');
    await user.click(row('Ticket 2'));
    await user.keyboard('{/Shift}');
    expect(onAction).not.toHaveBeenCalled();
    expect([...onChange.mock.lastCall![0]]).toEqual(['t2']);
  });

  it('Alt+Up/Down call the move callbacks with the focused row and keep focus', async () => {
    const onMoveUp = vi.fn();
    const onMoveDown = vi.fn();
    render(<Rows items={many(5)} onAction={() => {}} onMoveUp={onMoveUp} onMoveDown={onMoveDown} />);
    await userEvent.tab();
    await userEvent.keyboard('{ArrowDown}');
    expect(row('Ticket 1')).toHaveFocus();
    await userEvent.keyboard('{Alt>}{ArrowDown}{/Alt}');
    expect(onMoveDown).toHaveBeenCalledWith('t1');
    expect(row('Ticket 1')).toHaveFocus();
    await userEvent.keyboard('{Alt>}{ArrowUp}{/Alt}');
    expect(onMoveUp).toHaveBeenCalledWith('t1');
  });

  it('gives rows a drag handle when dragKind is set', () => {
    const { unmount } = render(<Rows items={many(3)} dragKind="ticket" onMove={() => {}} />);
    expect(row('Ticket 0').querySelector('[data-drag-handle]')).not.toBeNull();
    unmount();
    render(<Rows items={many(3)} />);
    expect(row('Ticket 0').querySelector('[data-drag-handle]')).toBeNull();
  });

  it('reports a rank move from a keyboard drag', async () => {
    const onMove = vi.fn();
    render(<Rows items={many(3)} dragKind="ticket" onMove={onMove} />);
    const handle = row('Ticket 0').querySelector<HTMLElement>('[data-drag-handle]')!;
    act(() => handle.focus());
    await userEvent.keyboard('{Enter}');
    const focused = () => document.activeElement?.getAttribute('aria-label') ?? '';
    await vi.waitFor(() => expect(focused()).toMatch(/^Insert/));
    for (let i = 0; i < 8 && focused() !== 'Insert between Ticket 1 and Ticket 2'; i++) await userEvent.keyboard('{ArrowDown}');
    expect(focused()).toBe('Insert between Ticket 1 and Ticket 2');
    await userEvent.keyboard('{Enter}');
    await vi.waitFor(() => expect(onMove).toHaveBeenCalledTimes(1));
    const move = onMove.mock.calls[0][0];
    expect(move.id).toBe('t0');
    expect(move.ids).toEqual(['t0']);
    expect([{ after: 't1' }, { before: 't2' }]).toContainEqual({ ...(move.after ? { after: move.after } : { before: move.before }) });
  });

  it('renders group headers with counts and collapses them', async () => {
    const onToggleSection = vi.fn();
    const sections: DataListSection<Row>[] = [
      { id: 'ready', title: 'Ready', items: many(3) },
      { id: 'draft', title: 'Draft', count: 40, collapsed: true, items: [{ id: 'd1', title: 'Hidden draft' }] },
    ];
    render(<Rows sections={sections} onToggleSection={onToggleSection} />);
    expect(screen.getByText('Ready').parentElement).toHaveTextContent('Ready3');
    expect(screen.getByText('Draft').parentElement).toHaveTextContent('Draft40');
    expect(screen.queryByRole('row', { name: 'Hidden draft' })).toBeNull();
    const expand = screen.getByRole('button', { name: 'Expand group' });
    expect(expand).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(expand);
    expect(onToggleSection).toHaveBeenCalledWith('draft');
  });

  it('asks for the next page near the end, but not while one is loading', () => {
    const onEndReached = vi.fn();
    observed = [];
    const { rerender } = render(<Rows items={many(3)} onEndReached={onEndReached} />);
    act(() => observed.at(-1)?.([{ isIntersecting: true }]));
    expect(onEndReached).toHaveBeenCalledTimes(1);
    rerender(<Rows items={many(3)} onEndReached={onEndReached} loadingMore />);
    act(() => observed.at(-1)?.([{ isIntersecting: true }]));
    expect(onEndReached).toHaveBeenCalledTimes(1);
  });

  it('shows the empty state', () => {
    render(<Rows items={[]} renderEmpty={() => <p>No tickets match.</p>} />);
    expect(within(screen.getByRole('grid')).getByText('No tickets match.')).toBeInTheDocument();
  });

  it('opens a row menu on right-click', () => {
    const onMenu = vi.fn();
    render(
      <DataList aria-label="Tickets" rowHeight={36} items={many(2)}>
        {(r) => <DataListRow id={r.id} textValue={r.title} onMenu={onMenu}>{r.title}</DataListRow>}
      </DataList>,
    );
    fireEvent.contextMenu(row('Ticket 1'), { clientX: 5, clientY: 6 });
    expect(onMenu).toHaveBeenCalledWith({ x: 5, y: 6 });
  });
});

describe('DataList touch long press', () => {
  it('opens the row menu without selecting the row', () => {
    vi.useFakeTimers();
    try {
      const onMenu = vi.fn();
      const changes: Set<string>[] = [];
      function MenuRows() {
        const [keys, setKeys] = useState<Set<string>>(new Set());
        return (
          <DataList aria-label="Tickets" rowHeight={36} items={many(3)} selectable selectedKeys={keys} onSelectionChange={(k) => { changes.push(k); setKeys(k); }} onAction={() => {}}>
            {(r) => (
              <DataListRow id={r.id} textValue={r.title} onMenu={onMenu}>
                <span>{r.title}</span>
              </DataListRow>
            )}
          </DataList>
        );
      }
      render(<MenuRows />);
      const target = row('Ticket 1');
      const touch = { pointerType: 'touch', pointerId: 1, button: 0, width: 1, height: 1, clientX: 5, clientY: 6 };
      fireEvent.pointerDown(target, touch);
      act(() => { vi.advanceTimersByTime(800); });
      fireEvent.pointerUp(target, touch);
      expect(onMenu).toHaveBeenCalledTimes(1);
      expect(changes.at(-1)?.size ?? 0).toBe(0);
      expect(target).not.toHaveAttribute('aria-selected', 'true');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('DataList striped', () => {
  it('fills every other row, counting from the top of each group', () => {
    const sections: DataListSection<Row>[] = [
      { id: 'a', title: 'A', items: many(3) },
      { id: 'b', title: 'B', items: [{ id: 'x0', title: 'X 0' }, { id: 'x1', title: 'X 1' }] },
    ];
    render(
      <DataList aria-label="Tickets" rowHeight={36} striped sections={sections}>
        {(r) => (
          <DataListRow id={r.id} textValue={r.title}>
            <span>{r.title}</span>
          </DataListRow>
        )}
      </DataList>,
    );
    const banded = (name: string) => row(name).className.includes('bg-band');
    expect([banded('Ticket 0'), banded('Ticket 1'), banded('Ticket 2')]).toEqual([false, true, false]);
    expect([banded('X 0'), banded('X 1')]).toEqual([false, true]);
  });

  it('leaves rows plain without it', () => {
    render(<Rows items={many(2)} />);
    expect(row('Ticket 1').className).not.toContain('bg-band');
  });
});
