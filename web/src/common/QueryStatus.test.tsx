import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { LoadError, QueryStatus } from './QueryStatus';

const q = <T,>(over: { data?: T; error?: Error | null; isPending?: boolean }) => ({
  data: undefined, error: null, isPending: false, refetch: vi.fn(), ...over,
});

describe('QueryStatus', () => {
  it('shows a labelled skeleton while loading', () => {
    render(<QueryStatus query={q({ isPending: true })} what="MCP servers" lines={2}>{() => 'x'}</QueryStatus>);
    expect(screen.getByRole('status', { name: 'Loading MCP servers' })).toBeInTheDocument();
    render(<QueryStatus query={q({ isPending: true })} what="Secrets">{() => 'x'}</QueryStatus>);
    expect(screen.getByRole('status', { name: 'Loading secrets' })).toBeInTheDocument();
  });

  it('says what could not be loaded, with Retry when asked', async () => {
    const query = q({ error: new Error('Gone') });
    render(<QueryStatus query={query} what="The run" retry>{() => 'x'}</QueryStatus>);
    expect(screen.getByRole('alert')).toHaveTextContent('The run could not be loaded: Gone');
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(query.refetch).toHaveBeenCalled();
  });

  it('keeps showing data that loaded before a failed refetch', () => {
    render(<QueryStatus query={q({ data: ['a'], error: new Error('Gone') })} what="Runs">{(d) => d.join()}</QueryStatus>);
    expect(screen.getByText('a')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('LoadError', () => {
  it('renders nothing without an error', () => {
    const { container } = render(<LoadError what="Notes" error={null} />);
    expect(container).toBeEmptyDOMElement();
  });
});
