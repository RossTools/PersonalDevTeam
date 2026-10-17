import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi } from '../test-utils';
import { ActivityBadge } from './ActivityBadge';
import { useViewTracking } from './useViewTracking';

vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => <div /> }));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const base: Session = {
  id: 's1', projectId: 'p', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Claude · PDT', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'idle', attentionAt: null, lastViewedAt: null, unseen: false,
};

describe('ActivityBadge', () => {
  it('names every activity and marks unseen ones', () => {
    const { rerender, container } = render(<ActivityBadge activity="working" unseen={false} />);
    expect(screen.getByText('Working')).toBeInTheDocument();
    rerender(<ActivityBadge activity="needs_you" unseen />);
    expect(screen.getByText('Needs you')).toBeInTheDocument();
    expect(screen.getByLabelText('unseen')).toBeInTheDocument();
    rerender(<ActivityBadge activity="idle" unseen={false} />);
    expect(screen.queryByLabelText('unseen')).not.toBeInTheDocument();
    rerender(<ActivityBadge activity="unknown" unseen={false} />);
    expect(screen.getByText('Unknown')).toBeInTheDocument();
    rerender(<ActivityBadge activity={null} unseen={false} />);
    expect(container).toBeEmptyDOMElement();
  });
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
