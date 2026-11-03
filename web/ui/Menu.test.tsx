import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Button } from './Button';
import { ContextMenu, MenuAt, MenuButton, MenuList, useMenuAt, type MenuEntry } from './Menu';
import { List, ListRow } from './List';

function entries(spies: Record<string, () => void> = {}): MenuEntry[] {
  return [
    { label: 'Rename', shortcut: 'R', onSelect: spies.rename ?? (() => {}) },
    { label: 'Move left', disabled: true, onSelect: () => {} },
    { heading: 'Move to workspace' },
    { label: 'Release', onSelect: spies.release ?? (() => {}) },
    { input: 'New workspace…', onSubmit: spies.submit ?? (() => {}) },
    { separator: true },
    { submenu: 'More', entries: [{ label: 'Deep item', onSelect: spies.deep ?? (() => {}) }] },
    { label: 'Close tab', danger: true, onSelect: spies.close ?? (() => {}) },
  ];
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('MenuButton', () => {
  it('opens a menu named by its label with every kind of entry', async () => {
    const user = userEvent.setup();
    render(<MenuButton label="Tab actions" entries={() => entries()} />);
    await user.click(screen.getByRole('button', { name: 'Tab actions' }));
    const menu = screen.getByRole('menu', { name: 'Tab actions' });
    expect(within(menu).getByRole('menuitem', { name: /Rename/ })).toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'Move left' })).toHaveAttribute('aria-disabled', 'true');
    expect(within(menu).getByText('Move to workspace')).toBeInTheDocument();
    expect(within(menu).getByRole('separator')).toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'Close tab' })).toHaveClass('text-destructive');
    // The shortcut is shown as a key.
    expect(within(menu).getByText('R').tagName).toBe('KBD');
  });

  it('moves with the arrow keys, selects with Enter, closes and returns focus', async () => {
    const user = userEvent.setup();
    const rename = vi.fn();
    const release = vi.fn();
    render(<MenuButton label="Tab actions" entries={() => entries({ rename, release })} />);
    const trigger = screen.getByRole('button', { name: 'Tab actions' });
    trigger.focus();
    await user.keyboard('{ArrowDown}');
    const menu = screen.getByRole('menu', { name: 'Tab actions' });
    expect(within(menu).getByRole('menuitem', { name: /Rename/ })).toHaveFocus();
    await user.keyboard('{ArrowDown}'); // skips the disabled item
    expect(within(menu).getByRole('menuitem', { name: 'Release' })).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(release).toHaveBeenCalledOnce();
    expect(rename).not.toHaveBeenCalled();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('closes on Escape and returns focus to the trigger', async () => {
    const user = userEvent.setup();
    render(<MenuButton label="Tab actions" entries={() => entries()} />);
    const trigger = screen.getByRole('button', { name: 'Tab actions' });
    await user.click(trigger);
    expect(screen.getByRole('menu')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('computes entries when it opens, not before', async () => {
    const user = userEvent.setup();
    const make = vi.fn(() => entries());
    render(<MenuButton label="Tab actions" entries={make} />);
    expect(make).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Tab actions' }));
    expect(make).toHaveBeenCalled();
  });

  it('opens a submenu and selects in it', async () => {
    const user = userEvent.setup();
    const deep = vi.fn();
    render(<MenuButton label="Tab actions" entries={() => entries({ deep })} />);
    await user.click(screen.getByRole('button', { name: 'Tab actions' }));
    await user.click(screen.getByRole('menuitem', { name: 'More' }));
    await user.click(within(screen.getByRole('menu', { name: 'More' })).getByRole('menuitem', { name: 'Deep item' }));
    expect(deep).toHaveBeenCalledOnce();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('turns an input entry into a text field and submits its value', async () => {
    const user = userEvent.setup();
    const submit = vi.fn();
    render(<MenuButton label="Tab actions" entries={() => entries({ submit })} />);
    await user.click(screen.getByRole('button', { name: 'Tab actions' }));
    await user.click(screen.getByRole('menuitem', { name: 'New workspace…' }));
    const field = screen.getByRole('textbox', { name: 'New workspace…' });
    expect(field).toHaveFocus();
    await user.keyboard('  Later  {Enter}');
    expect(submit).toHaveBeenCalledWith('Later');
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  });

  it('accepts a custom trigger', async () => {
    const user = userEvent.setup();
    render(
      <MenuButton label="Run actions" entries={() => entries()}>
        <button type="button">Run ▾</button>
      </MenuButton>,
    );
    await user.click(screen.getByRole('button', { name: 'Run ▾' }));
    expect(screen.getByRole('menu', { name: 'Run actions' })).toBeInTheDocument();
  });
});

describe('MenuList', () => {
  it('shows the entries in place, ticks the chosen one, moves with the arrow keys and selects with Enter', async () => {
    const user = userEvent.setup();
    const weekly = vi.fn();
    render(
      <MenuList
        label="Repeat"
        entries={[
          { label: 'Daily', detail: 'every day', checked: true, onSelect: () => {} },
          { label: 'Weekly', detail: 'on Fri', onSelect: weekly },
        ]}
      />,
    );
    const menu = screen.getByRole('menu', { name: 'Repeat' });
    const [daily, week] = within(menu).getAllByRole('menuitem');
    expect(within(daily).getByLabelText('Chosen')).toBeInTheDocument();
    expect(daily).not.toHaveTextContent('every day');
    expect(week).toHaveTextContent('on Fri');
    act(() => daily.focus());
    await user.keyboard('{ArrowDown}');
    expect(week).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(weekly).toHaveBeenCalledTimes(1);
    expect(menu).toBeInTheDocument();
  });
});

describe('ContextMenu', () => {
  function Tab({ onClick = () => {}, make = () => entries() }: { onClick?: () => void; make?: () => MenuEntry[] }) {
    return (
      <ContextMenu label="Tab Claude A" entries={make}>
        <button type="button" onClick={onClick}>
          Claude A
        </button>
      </ContextMenu>
    );
  }

  it('opens on right-click, lazily, and selects', async () => {
    const user = userEvent.setup();
    const close = vi.fn();
    const make = vi.fn(() => entries({ close }));
    render(<Tab make={make} />);
    expect(make).not.toHaveBeenCalled();
    fireEvent.contextMenu(screen.getByRole('button', { name: 'Claude A' }));
    const menu = screen.getByRole('menu', { name: 'Tab Claude A' });
    expect(make).toHaveBeenCalled();
    await user.click(within(menu).getByRole('menuitem', { name: 'Close tab' }));
    expect(close).toHaveBeenCalledOnce();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('a left click does not open it, and the element keeps its own click (mouse and keyboard)', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(<Tab onClick={onClick} />);
    const tab = screen.getByRole('button', { name: 'Claude A' });
    await user.click(tab);
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    tab.focus();
    await user.keyboard('{Enter}');
    expect(onClick).toHaveBeenCalledTimes(2);
  });

  it('closes on Escape and returns focus to the element', async () => {
    const user = userEvent.setup();
    render(<Tab />);
    const tab = screen.getByRole('button', { name: 'Claude A' });
    tab.focus();
    fireEvent.contextMenu(tab);
    const menu = screen.getByRole('menu');
    await waitFor(() => expect(menu).toContainElement(document.activeElement as HTMLElement));
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    await waitFor(() => expect(tab).toHaveFocus());
  });

  it('works on a React Aria based child (our Button) without breaking its click', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <ContextMenu label="Row menu" entries={() => entries()}>
        <Button onClick={onClick}>Row</Button>
      </ContextMenu>,
    );
    const row = screen.getByRole('button', { name: 'Row' });
    await user.click(row);
    expect(onClick).toHaveBeenCalledOnce();
    fireEvent.contextMenu(row);
    expect(screen.getByRole('menu', { name: 'Row menu' })).toBeInTheDocument();
  });

  it('does nothing when disabled', () => {
    render(
      <ContextMenu label="Row menu" entries={() => entries()} disabled>
        <button type="button">Row</button>
      </ContextMenu>,
    );
    fireEvent.contextMenu(screen.getByRole('button', { name: 'Row' }));
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('opens on a long press (touch)', () => {
    // Android fires `contextmenu` on a long press (covered above); iOS doesn't, so React Aria runs its own timer there.
    vi.spyOn(navigator, 'platform', 'get').mockReturnValue('iPhone');
    vi.useFakeTimers();
    render(<Tab />);
    const tab = screen.getByRole('button', { name: 'Claude A' });
    fireEvent.pointerDown(tab, { pointerType: 'touch', pointerId: 1, button: 0, width: 1, height: 1 });
    act(() => {
      vi.advanceTimersByTime(600);
    });
    fireEvent.pointerUp(tab, { pointerType: 'touch', pointerId: 1, button: 0, width: 1, height: 1 });
    expect(screen.getByRole('menu', { name: 'Tab Claude A' })).toBeInTheDocument();
  });
});

describe('MenuAt', () => {
  it('opens at the point, fires an item and closes', async () => {
    const user = userEvent.setup();
    const release = vi.fn();
    const onClose = vi.fn();
    render(<MenuAt open at={{ x: 120, y: 80 }} label="Agent X" entries={entries({ release })} onClose={onClose} />);
    const menu = screen.getByRole('menu', { name: 'Agent X' });
    // Anchored to a zero-size element at the point (jsdom has no layout, so check the anchor, not the menu).
    const anchor = document.querySelector<HTMLElement>('span[aria-hidden][style*="position: fixed"]');
    expect(anchor?.style.left).toBe('120px');
    expect(anchor?.style.top).toBe('80px');
    await user.click(within(menu).getByRole('menuitem', { name: 'Release' }));
    expect(release).toHaveBeenCalledOnce();
    expect(onClose).toHaveBeenCalled();
  });

  it('closes on Escape', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<MenuAt open at={{ x: 0, y: 0 }} label="Agent X" entries={() => entries()} onClose={onClose} />);
    const menu = screen.getByRole('menu', { name: 'Agent X' });
    await waitFor(() => expect(menu).toContainElement(document.activeElement as HTMLElement));
    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
  });

  it('useMenuAt: a ListRow onMenu opens it, focus returns to the row', async () => {
    const user = userEvent.setup();
    const close = vi.fn();
    function Rows() {
      const { openMenu, menu } = useMenuAt();
      return (
        <>
          <List aria-label="Agents">
            <ListRow id="a" title="Agent A" onMenu={(at) => openMenu(at, 'Agent A', () => entries({ close }))} />
            <ListRow id="b" title="Agent B" />
          </List>
          {menu}
        </>
      );
    }
    render(<Rows />);
    const row = screen.getByRole('row', { name: /Agent A/ });
    row.focus();
    fireEvent.contextMenu(row, { clientX: 30, clientY: 40 });
    const menu = screen.getByRole('menu', { name: 'Agent A' });
    await waitFor(() => expect(menu).toContainElement(document.activeElement as HTMLElement));
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    await waitFor(() => expect(row).toHaveFocus());
    fireEvent.contextMenu(row, { clientX: 30, clientY: 40 });
    await user.click(screen.getByRole('menuitem', { name: 'Close tab' }));
    expect(close).toHaveBeenCalledOnce();
  });
});
