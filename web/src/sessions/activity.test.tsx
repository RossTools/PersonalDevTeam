import { act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { useViewTracking } from './useViewTracking';

vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => <div /> }));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function Probe({ id, unseen }: { id: string; unseen: boolean }) {
  useViewTracking(id, unseen);
  return null;
}

describe('useViewTracking', () => {
  it('posts on mount and when a poll finds the session unseen, only while the tab is visible', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/view': undefined });
    let visibility = 'visible';
    vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility as DocumentVisibilityState);
    const { rerender } = renderWithQuery(<Probe id="s1" unseen={false} />);
    await waitFor(() => expect(calls).toHaveLength(1));
    // Polls that find nothing new post nothing.
    rerender(<Probe id="s1" unseen={false} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toHaveLength(1);
    rerender(<Probe id="s1" unseen />);
    await waitFor(() => expect(calls).toHaveLength(2));
    rerender(<Probe id="s1" unseen={false} />);
    visibility = 'hidden';
    rerender(<Probe id="s1" unseen />);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toHaveLength(2);
    visibility = 'visible';
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    await waitFor(() => expect(calls).toHaveLength(3));
  });
});
