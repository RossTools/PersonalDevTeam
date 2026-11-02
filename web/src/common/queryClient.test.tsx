import { act, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useMutation } from '@tanstack/react-query';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { toast } from '@ui/Toaster';
import { renderWithQuery } from '../test-utils';

beforeAll(() => {
  Element.prototype.setPointerCapture ??= () => {};
  Element.prototype.releasePointerCapture ??= () => {};
  window.matchMedia ??= ((query: string) => ({
    matches: false, media: query, onchange: null,
    addEventListener: () => {}, removeEventListener: () => {}, addListener: () => {}, removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
});

afterEach(() => act(() => { toast.dismiss(); }));

const fail = () => Promise.reject(new Error('The save failed'));

function Silent() {
  const m = useMutation({ mutationFn: fail });
  return <button type="button" onClick={() => m.mutate()}>{m.isError ? 'Failed' : 'Save'}</button>;
}

function Inline() {
  const m = useMutation({ meta: { errorShown: true }, mutationFn: fail });
  return (
    <>
      <button type="button" onClick={() => m.mutate()}>Save</button>
      {m.error && <p role="alert">{m.error.message}</p>}
    </>
  );
}

function OwnHandler({ onError }: { onError: (e: Error) => void }) {
  const m = useMutation({ mutationFn: fail, onError });
  return <button type="button" onClick={() => m.mutate()}>{m.isError ? 'Failed' : 'Save'}</button>;
}

describe('the fallback mutation toast', () => {
  it('toasts the failure of a mutation that handles no error', async () => {
    renderWithQuery(<Silent />);
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('The save failed')).toBeInTheDocument();
  });

  it('does not toast a mutation that renders its error inline', async () => {
    renderWithQuery(<Inline />);
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    // Only the inline error: a toast would make it appear twice.
    expect(await screen.findByRole('alert')).toHaveTextContent('The save failed');
    expect(screen.getAllByText('The save failed')).toHaveLength(1);
  });

  it('does not toast a mutation with its own onError', async () => {
    const seen: string[] = [];
    renderWithQuery(<OwnHandler onError={(e) => seen.push(e.message)} />);
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await screen.findByRole('button', { name: 'Failed' });
    expect(seen).toEqual(['The save failed']);
    expect(screen.queryByText('The save failed')).not.toBeInTheDocument();
  });
});
