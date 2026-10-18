import { act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { useViewTracking } from './useViewTracking';

vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => <div /> }));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function Probe({ id, polledAt }: { id: string; polledAt: number }) {
  useViewTracking(id, polledAt);
  return null;
}

describe('useViewTracking', () => {
  it('posts views while the tab is visible, including on each poll', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/view': undefined });
    let visibility = 'visible';
    vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility as DocumentVisibilityState);
    const { rerender } = renderWithQuery(<Probe id="s1" polledAt={1} />);
    await waitFor(() => expect(calls).toHaveLength(1));
    rerender(<Probe id="s1" polledAt={2} />);
    await waitFor(() => expect(calls).toHaveLength(2));
    visibility = 'hidden';
    rerender(<Probe id="s1" polledAt={3} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toHaveLength(2);
    visibility = 'visible';
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    await waitFor(() => expect(calls).toHaveLength(3));
  });
});
