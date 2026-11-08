import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RowLongPress, RowMenus, openFocusedRowMenu, useRowMenu, type MenuPoint } from './rowMenu';

afterEach(() => vi.useRealTimers());

function Row({ id, onMenu }: { id: string; onMenu?: (p: MenuPoint) => void }) {
  return <div role="row" tabIndex={0} data-key={id} aria-label={id} {...useRowMenu(id, onMenu)} />;
}

/** Rows in a list that opens the focused row's menu from the keyboard, as List and DataList do. */
function Rows({ menus, rows, onLongPress }: { menus: Map<string, (p: MenuPoint) => void>; rows: { id: string; onMenu?: (p: MenuPoint) => void }[]; onLongPress?: (id: string) => void }) {
  return (
    <RowMenus.Provider value={menus}>
      <RowLongPress.Provider value={onLongPress ?? null}>
        <div role="grid" onKeyDown={(e) => openFocusedRowMenu(e, menus)}>
          {rows.map((r) => <Row key={r.id} {...r} />)}
        </div>
      </RowLongPress.Provider>
    </RowMenus.Provider>
  );
}

describe('row menus', () => {
  it('Shift+F10 and the Menu key open the focused row’s menu below it; other keys and rows without one do not', () => {
    const onMenu = vi.fn();
    render(<Rows menus={new Map()} rows={[{ id: 'a', onMenu }, { id: 'b' }]} />);
    const a = screen.getByRole('row', { name: 'a' });
    vi.spyOn(a, 'getBoundingClientRect').mockReturnValue({ left: 10, bottom: 50 } as DOMRect);
    fireEvent.keyDown(a, { key: 'F10' });
    expect(onMenu).not.toHaveBeenCalled();
    fireEvent.keyDown(a, { key: 'F10', shiftKey: true });
    expect(onMenu).toHaveBeenLastCalledWith({ x: 18, y: 50 });
    fireEvent.keyDown(a, { key: 'ContextMenu' });
    expect(onMenu).toHaveBeenCalledTimes(2);
    // A row without a menu: the key goes on as it was (not prevented).
    expect(fireEvent.keyDown(screen.getByRole('row', { name: 'b' }), { key: 'ContextMenu' })).toBe(true);
    expect(onMenu).toHaveBeenCalledTimes(2);
  });

  it('a row registers its menu while it shows, and only its own', () => {
    const menus = new Map<string, (p: MenuPoint) => void>();
    const first = vi.fn();
    const { rerender, unmount } = render(<Rows menus={menus} rows={[{ id: 'a', onMenu: first }]} />);
    expect(menus.get('a')).toBe(first);
    const next = vi.fn();
    rerender(<Rows menus={menus} rows={[{ id: 'a', onMenu: next }]} />);
    expect(menus.get('a')).toBe(next);
    unmount();
    expect(menus.size).toBe(0);
  });

  it('a mouse press never opens it by holding; a right-click does, where it was clicked', () => {
    vi.useFakeTimers();
    const onMenu = vi.fn();
    render(<Rows menus={new Map()} rows={[{ id: 'a', onMenu }]} />);
    const row = screen.getByRole('row', { name: 'a' });
    fireEvent.pointerDown(row, { pointerType: 'mouse', clientX: 1, clientY: 1 });
    act(() => vi.advanceTimersByTime(1000));
    expect(onMenu).not.toHaveBeenCalled();
    fireEvent.contextMenu(row, { clientX: 7, clientY: 8 });
    expect(onMenu).toHaveBeenCalledWith({ x: 7, y: 8 });
  });

  it('a touch that moves (a scroll) is no long press; one that stays still is, and the list hears of it', () => {
    vi.useFakeTimers();
    const onMenu = vi.fn();
    const onLongPress = vi.fn();
    render(<Rows menus={new Map()} rows={[{ id: 'a', onMenu }]} onLongPress={onLongPress} />);
    const row = screen.getByRole('row', { name: 'a' });
    fireEvent.pointerDown(row, { pointerType: 'touch', clientX: 5, clientY: 5 });
    fireEvent.pointerMove(row, { pointerType: 'touch', clientX: 5, clientY: 30 });
    act(() => vi.advanceTimersByTime(600));
    expect(onMenu).not.toHaveBeenCalled();
    fireEvent.pointerDown(row, { pointerType: 'touch', clientX: 5, clientY: 5 });
    // A wobble of a few pixels still counts as holding still.
    fireEvent.pointerMove(row, { pointerType: 'touch', clientX: 8, clientY: 9 });
    act(() => vi.advanceTimersByTime(600));
    expect(onMenu).toHaveBeenCalledWith({ x: 5, y: 5 });
    expect(onLongPress).toHaveBeenCalledWith('a');
  });

  it('a row without a menu gets no handlers', () => {
    const menus = new Map<string, (p: MenuPoint) => void>();
    render(<Rows menus={menus} rows={[{ id: 'a' }]} />);
    const row = screen.getByRole('row', { name: 'a' });
    // The browser's own menu: not prevented.
    expect(fireEvent.contextMenu(row)).toBe(true);
    expect(menus.size).toBe(0);
  });
});
