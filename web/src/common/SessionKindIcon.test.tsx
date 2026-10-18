import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { SessionKindIcon, sessionKindLabel } from './SessionKindIcon';

describe('SessionKindIcon', () => {
  it('names what the session runs', () => {
    expect(sessionKindLabel({ kind: 'agent_tui', agent: 'claude' })).toBe('Claude');
    expect(sessionKindLabel({ kind: 'agent_tui', agent: 'codex' })).toBe('Codex');
    expect(sessionKindLabel({ kind: 'shell', agent: null })).toBe('Shell');
  });

  it('is a labelled image, and nothing without a session', () => {
    const { container, rerender } = render(<SessionKindIcon session={{ kind: 'agent_tui', agent: 'codex' }} />);
    expect(screen.getByRole('img', { name: 'Codex' })).toHaveAttribute('title', 'Codex');
    rerender(<SessionKindIcon session={undefined} />);
    expect(container).toBeEmptyDOMElement();
  });
});
