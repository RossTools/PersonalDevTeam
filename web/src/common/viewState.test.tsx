import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  clearViewStates, pruneTabViewStates, readViewState, tabScope, useRememberedScroll, useViewState, ViewScopeContext, writeViewState,
} from './viewState';

afterEach(() => vi.useRealTimers());

function Counter({ name = 'count' }: { name?: string }) {
  const [n, setN] = useViewState(name, 0);
  return <button type="button" onClick={() => setN((x) => x + 1)}>{n}</button>;
}

function Scroller() {
  const { ref, onScroll } = useRememberedScroll('list');
  return <div ref={ref} onScroll={onScroll} data-testid="scroller" />;
}

describe('the view-state store', () => {
  it('gives a view back its state when it mounts again in the same scope', async () => {
    const inTab = (tab: string) => <ViewScopeContext.Provider value={tabScope(tab)}><Counter /></ViewScopeContext.Provider>;
    const first = render(inTab('t1'));
    await userEvent.click(screen.getByRole('button'));
    await userEvent.click(screen.getByRole('button'));
    expect(screen.getByRole('button')).toHaveTextContent('2');
    first.unmount();

    // Another tab has its own.
    const other = render(inTab('t2'));
    expect(screen.getByRole('button')).toHaveTextContent('0');
    other.unmount();

    render(inTab('t1'));
    expect(screen.getByRole('button')).toHaveTextContent('2');
  });

  it('mirrors entries to sessionStorage, so a fresh page reads them', () => {
    vi.useFakeTimers();
    writeViewState('tab:a|x', { file: 'README.md' });
    act(() => { vi.advanceTimersByTime(500); });
    expect(JSON.parse(sessionStorage.getItem('pdt.view:tab:a|x')!)).toEqual({ file: 'README.md' });
    // Memory gone (a reload): read back from storage.
    const raw = sessionStorage.getItem('pdt.view:tab:a|x')!;
    clearViewStates();
    sessionStorage.setItem('pdt.view:tab:a|x', raw);
    expect(readViewState('tab:a|x')).toEqual({ file: 'README.md' });
  });

  it('drops the entries of tabs that closed, and keeps the rest', () => {
    vi.useFakeTimers();
    writeViewState('tab:open|x', 1);
    writeViewState('tab:closed|x', 2);
    writeViewState('view:tickets|scroll', 3);
    act(() => { vi.advanceTimersByTime(500); });
    pruneTabViewStates(['open']);
    act(() => { vi.advanceTimersByTime(500); });
    expect(readViewState('tab:open|x')).toBe(1);
    expect(readViewState('tab:closed|x')).toBeUndefined();
    expect(sessionStorage.getItem('pdt.view:tab:closed|x')).toBeNull();
    expect(readViewState('view:tickets|scroll')).toBe(3);
  });

  it('remembers a scroll position and puts it back on mount', () => {
    const first = render(<Scroller />);
    const el = screen.getByTestId('scroller');
    el.scrollTop = 340;
    fireEvent.scroll(el);
    first.unmount();
    render(<Scroller />);
    expect(screen.getByTestId('scroller').scrollTop).toBe(340);
  });
});
