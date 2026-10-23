import { act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { useViewTracking } from './useViewTracking';

vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => <div /> }));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function Probe({ id, unseenAt }: { id: string; unseenAt: number | null }) {
  useViewTracking(id, unseenAt);
  return null;
}

describe('useViewTracking', () => {
  it('posts on mount and when a poll finds new unseen attention, only while the tab is visible', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/view': undefined });
    let visibility = 'visible';
    vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility as DocumentVisibilityState);
    const { rerender } = renderWithQuery(<Probe id="s1" unseenAt={null} />);
    await waitFor(() => expect(calls).toHaveLength(1));
    // Polls that find nothing new post nothing.
    rerender(<Probe id="s1" unseenAt={null} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toHaveLength(1);
    rerender(<Probe id="s1" unseenAt={100} />);
    await waitFor(() => expect(calls).toHaveLength(2));
    // Another turn ends before a poll shows it seen: still unseen, newer attention, posted again.
    rerender(<Probe id="s1" unseenAt={200} />);
    await waitFor(() => expect(calls).toHaveLength(3));
    rerender(<Probe id="s1" unseenAt={200} />);
    rerender(<Probe id="s1" unseenAt={null} />);
    visibility = 'hidden';
    rerender(<Probe id="s1" unseenAt={300} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toHaveLength(3);
    visibility = 'visible';
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    await waitFor(() => expect(calls).toHaveLength(4));
  });
});
