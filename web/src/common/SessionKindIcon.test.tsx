import { screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { SessionKindIcon, sessionKindLabel } from './SessionKindIcon';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('SessionKindIcon', () => {
  it('names what the session runs', () => {
    expect(sessionKindLabel({ kind: 'agent_tui', agent: 'claude' })).toBe('Claude');
    expect(sessionKindLabel({ kind: 'agent_tui', agent: 'codex' })).toBe('Codex');
    expect(sessionKindLabel({ kind: 'agent_tui', agent: 'devin' })).toBe('Devin');
    expect(sessionKindLabel({ kind: 'agent_tui', agent: 'acp:fake' })).toBe('ACP agent');
    expect(sessionKindLabel({ kind: 'shell', agent: null })).toBe('Shell');
  });

  it('is a labelled image, and nothing without a session', () => {
    stubApi({});
    const { container, rerender } = renderWithQuery(<SessionKindIcon session={{ kind: 'agent_tui', agent: 'codex' }} />);
    expect(screen.getByRole('img', { name: 'Codex' })).toHaveAttribute('title', 'Codex');
    rerender(<SessionKindIcon session={undefined} />);
    expect(container.querySelector('[role="img"]')).toBeNull();
  });

  it("names an ACP agent's session by the agent's label", async () => {
    stubApi({ 'GET /api/host/settings': { acpAgents: [{ agent: 'acp:fake', label: 'Fake ACP' }] } });
    renderWithQuery(<SessionKindIcon session={{ kind: 'agent_tui', agent: 'acp:fake' }} />);
    expect(await screen.findByRole('img', { name: 'Fake ACP' })).toBeInTheDocument();
  });
});
