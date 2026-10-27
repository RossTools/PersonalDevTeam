import { render, screen } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DuePicker } from './DuePicker';

function stubPointer(coarse: boolean) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: coarse && query === '(pointer: coarse)',
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

function Picker() {
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={anchor} type="button">Date</button>
      <DuePicker
        open
        onOpenChange={() => {}}
        anchor={anchor}
        value={{ dueOn: null, dueTime: null, recurrence: null }}
        now={new Date(2026, 9, 27, 10)}
        onChange={() => {}}
        onCustomRepeat={() => {}}
      />
    </>
  );
}

describe('DuePicker', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('focuses the type-a-date box with a mouse', () => {
    stubPointer(false);
    render(<Picker />);
    expect(screen.getByRole('textbox', { name: 'Type a date' })).toHaveFocus();
  });

  it('leaves the type-a-date box alone on a touch screen, so the keyboard stays shut', () => {
    stubPointer(true);
    render(<Picker />);
    expect(screen.getByRole('dialog', { name: 'Due date' })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Type a date' })).not.toHaveFocus();
  });
});
