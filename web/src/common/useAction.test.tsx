import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { Toaster, toast } from '@ui/Toaster';
import { ApiError } from '../api/client';
import { actionError, runAction, useAction, useFormAction } from './useAction';

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

function CloseButton({ fn, success }: { fn: () => Promise<unknown>; success?: string }) {
  const [run, pending] = useAction(fn, { success });
  return <button type="button" onClick={() => void run()}>{pending ? 'Closing' : 'Close'}</button>;
}

describe('useAction', () => {
  it('toasts the message of a rejected action and does not throw', async () => {
    render(<><Toaster /><CloseButton fn={() => Promise.reject(new ApiError(409, 'busy', 'The tab is busy'))} /></>);
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(await screen.findByText('The tab is busy')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();
  });

  it('is pending until the action settles, then toasts success', async () => {
    let resolve!: () => void;
    const fn = vi.fn(() => new Promise<void>((r) => { resolve = r; }));
    render(<><Toaster /><CloseButton fn={fn} success="Closed" /></>);
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.getByRole('button', { name: 'Closing' })).toBeInTheDocument();
    await act(async () => { resolve(); });
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();
    expect(await screen.findByText('Closed')).toBeInTheDocument();
    expect(fn).toHaveBeenCalledOnce();
  });
});

function SaveForm({ fn }: { fn: () => Promise<unknown> }) {
  const save = useFormAction(fn);
  return (
    <>
      <button type="button" onClick={() => { void save.run(); void save.run(); }}>{save.pending ? 'Saving' : 'Save'}</button>
      {save.error && <p role="alert">{save.error}</p>}
    </>
  );
}

describe('useFormAction', () => {
  it('shows the error inline, without a toast, and clears it on the next run', async () => {
    let fail = true;
    const fn = vi.fn(async () => { if (fail) throw new ApiError(400, 'bad', 'Title is required'); });
    render(<><Toaster /><SaveForm fn={fn} /></>);
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Title is required');
    expect(document.querySelector('[data-sonner-toast]')).toBeNull();
    fail = false;
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('is pending until the action settles and ignores a second run meanwhile', async () => {
    let resolve!: () => void;
    const fn = vi.fn(() => new Promise<void>((r) => { resolve = r; }));
    render(<SaveForm fn={fn} />);
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByRole('button', { name: 'Saving' })).toBeInTheDocument();
    expect(fn).toHaveBeenCalledOnce();
    await act(async () => { resolve(); });
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
  });
});

describe('runAction', () => {
  it('resolves to the result, or undefined with a toast on failure', async () => {
    render(<Toaster />);
    expect(await runAction(async () => 42)).toBe(42);
    let r: unknown = 'x';
    await act(async () => { r = await runAction(() => { throw new Error('Nope'); }); });
    expect(r).toBeUndefined();
    expect(await screen.findByText('Nope')).toBeInTheDocument();
  });
});

describe('actionError', () => {
  it('uses the message, else a generic one', () => {
    expect(actionError(new Error('Gone'))).toBe('Gone');
    expect(actionError('plain')).toBe('plain');
    expect(actionError(undefined)).toBe('Something went wrong');
  });
});
