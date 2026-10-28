import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ErrorBoundary } from './ErrorBoundary';
import { isChunkLoadError, reloadForNewBuild } from './staleBuild';

afterEach(() => vi.restoreAllMocks());

let fail = true;
function View() {
  if (fail) throw new Error('boom');
  return <p>The view</p>;
}

describe('ErrorBoundary', () => {
  it('shows the fallback with the error, logs it, and tries again on a new location', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    fail = true;
    const { rerender } = render(<ErrorBoundary resetKey="/inbox/a" onBack={() => {}}><View /></ErrorBoundary>);
    expect(screen.getByRole('alert')).toHaveTextContent('Something went wrong');
    expect(screen.getByRole('alert')).toHaveTextContent('boom');
    expect(console.error).toHaveBeenCalledWith('A view failed to render:', expect.any(Error), expect.anything());
    fail = false;
    rerender(<ErrorBoundary resetKey="/inbox/b" onBack={() => {}}><View /></ErrorBoundary>);
    expect(screen.getByText('The view')).toBeInTheDocument();
  });

  it('Back to Inbox goes back and shows the content again', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    fail = true;
    const onBack = vi.fn(() => { fail = false; });
    render(<ErrorBoundary onBack={onBack}><View /></ErrorBoundary>);
    await userEvent.click(screen.getByRole('button', { name: 'Back to Inbox' }));
    expect(onBack).toHaveBeenCalled();
    expect(screen.getByText('The view')).toBeInTheDocument();
  });

  it('says a newer version is out when a lazy view fails to load', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    function Stale(): never { throw new TypeError('Failed to fetch dynamically imported module: /assets/PhoneInbox-x.js'); }
    render(<ErrorBoundary onBack={() => {}}><Stale /></ErrorBoundary>);
    expect(screen.getByRole('alert')).toHaveTextContent('A newer version is out since this page loaded');
  });
});

describe('stale builds', () => {
  it("knows each browser's chunk load errors", () => {
    expect(isChunkLoadError(new TypeError('Failed to fetch dynamically imported module: http://x/a.js'))).toBe(true);
    expect(isChunkLoadError(new TypeError('Importing a module script failed.'))).toBe(true);
    expect(isChunkLoadError(new TypeError('error loading dynamically imported module: http://x/a.js'))).toBe(true);
    expect(isChunkLoadError(new TypeError("Cannot read properties of null (reading 'map')"))).toBe(false);
  });

  it('reloads once, not again within half a minute', () => {
    sessionStorage.clear();
    const reload = vi.fn();
    expect(reloadForNewBuild(sessionStorage, reload, 1_000_000)).toBe(true);
    expect(reloadForNewBuild(sessionStorage, reload, 1_010_000)).toBe(false);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(reloadForNewBuild(sessionStorage, reload, 1_040_000)).toBe(true);
    expect(reload).toHaveBeenCalledTimes(2);
  });
});
