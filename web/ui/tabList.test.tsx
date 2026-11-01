import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import { tabIndexOf, tabListKeyDown } from './tabList';

function Tabs({ disabled }: { disabled?: string }) {
  const [value, setValue] = useState('b');
  const ids = ['a', 'b', 'c', 'd'];
  return (
    <div role="tablist" aria-label="Sections" onKeyDown={tabListKeyDown}>
      {ids.map((id, i) => (
        <button
          key={id}
          type="button"
          role="tab"
          aria-selected={id === value}
          disabled={id === disabled}
          tabIndex={tabIndexOf(id === value, i, true)}
          onClick={() => setValue(id)}
        >
          {id}
        </button>
      ))}
      <input aria-label="Rename" />
    </div>
  );
}

const tab = (name: string) => screen.getByRole('tab', { name });

describe('tabListKeyDown', () => {
  it('moves focus and selection with the arrow keys, wrapping', () => {
    render(<Tabs />);
    tab('b').focus();
    fireEvent.keyDown(tab('b'), { key: 'ArrowRight' });
    expect(tab('c')).toHaveFocus();
    expect(tab('c')).toHaveAttribute('aria-selected', 'true');
    fireEvent.keyDown(tab('c'), { key: 'ArrowRight' });
    fireEvent.keyDown(tab('d'), { key: 'ArrowRight' });
    expect(tab('a')).toHaveFocus();
    fireEvent.keyDown(tab('a'), { key: 'ArrowLeft' });
    expect(tab('d')).toHaveAttribute('aria-selected', 'true');
  });

  it('goes to the ends with Home and End', () => {
    render(<Tabs />);
    fireEvent.keyDown(tab('b'), { key: 'End' });
    expect(tab('d')).toHaveFocus();
    fireEvent.keyDown(tab('d'), { key: 'Home' });
    expect(tab('a')).toHaveAttribute('aria-selected', 'true');
  });

  it('skips disabled tabs', () => {
    render(<Tabs disabled="c" />);
    fireEvent.keyDown(tab('b'), { key: 'ArrowRight' });
    expect(tab('d')).toHaveFocus();
  });

  it('leaves keys alone off a tab, and with a modifier', () => {
    render(<Tabs />);
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Rename' }), { key: 'ArrowRight' });
    fireEvent.keyDown(tab('b'), { key: 'ArrowRight', ctrlKey: true });
    expect(tab('b')).toHaveAttribute('aria-selected', 'true');
  });

  it('makes the selected tab the one Tab stop', () => {
    render(<Tabs />);
    expect(tab('b')).toHaveAttribute('tabindex', '0');
    expect(tab('a')).toHaveAttribute('tabindex', '-1');
    expect(tabIndexOf(false, 0, false)).toBe(0);
    expect(tabIndexOf(false, 1, false)).toBe(-1);
  });
});
