import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AutoSaveRow, AutoSaveText } from './AutoSave';

afterEach(() => vi.useRealTimers());

/** A field over a stored value that a save changes, as a settings page's query would. */
function Stored({ initial = 'old', onSave, multiline }: { initial?: string; onSave: (t: string) => Promise<unknown>; multiline?: boolean }) {
  const [value, setValue] = useState(initial);
  return <AutoSaveText aria-label="Name" value={value} multiline={multiline} onSave={async (t) => { await onSave(t); setValue(t); }} />;
}

describe('AutoSaveText', () => {
  it('saves the trimmed text on Enter, says Saved, and the Saved goes after a moment', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<Stored onSave={onSave} />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    await userEvent.clear(field);
    await userEvent.type(field, '  new  {Enter}');
    expect(onSave).toHaveBeenCalledWith('new');
    expect(await screen.findByRole('status')).toHaveTextContent('Saved');
    expect(field).toHaveValue('new');
    act(() => vi.advanceTimersByTime(2000));
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('saves on blur, and not when the text is the stored one', async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<Stored onSave={onSave} />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    await userEvent.type(field, ' ');
    fireEvent.blur(field);
    expect(onSave).not.toHaveBeenCalled();
    await userEvent.type(field, 'er');
    fireEvent.blur(field);
    await screen.findByRole('status');
    expect(onSave).toHaveBeenCalledWith('older');
  });

  it('Escape puts the stored value back', async () => {
    const onSave = vi.fn();
    render(<Stored onSave={onSave} />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    await userEvent.type(field, 'x');
    expect(field).toHaveValue('oldx');
    await userEvent.keyboard('{Escape}');
    expect(field).toHaveValue('old');
    fireEvent.blur(field);
    expect(onSave).not.toHaveBeenCalled();
  });

  it('a failed save shows its error and keeps the typed text to fix', async () => {
    render(<Stored onSave={() => Promise.reject(new Error('Name taken'))} />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    await userEvent.type(field, '2{Enter}');
    expect(await screen.findByRole('alert')).toHaveTextContent('Name taken');
    expect(field).toHaveValue('old2');
  });

  it('keeps typed text while the stored value changes under it', async () => {
    const { rerender } = render(<AutoSaveText aria-label="Name" value="a" onSave={vi.fn()} />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    await userEvent.type(field, 'b');
    rerender(<AutoSaveText aria-label="Name" value="polled" onSave={vi.fn()} />);
    expect(field).toHaveValue('ab');
  });

  it('a text area takes Enter as a new line and saves on Ctrl/⌘+Enter', async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<Stored onSave={onSave} multiline />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    await userEvent.type(field, '{Enter}two');
    expect(onSave).not.toHaveBeenCalled();
    expect(field).toHaveValue('old\ntwo');
    await userEvent.keyboard('{Control>}{Enter}{/Control}');
    expect(onSave).toHaveBeenCalledWith('old\ntwo');
    await userEvent.type(field, '!');
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}');
    expect(onSave).toHaveBeenLastCalledWith('old\ntwo!');
  });
});

describe('AutoSaveRow', () => {
  it("follows its control's save: Saved, or the error", async () => {
    let fail = false;
    render(
      <AutoSaveRow label="Theme">
        {(track) => <button type="button" onClick={() => void track(fail ? Promise.reject(new Error('Not allowed')) : Promise.resolve())}>Toggle</button>}
      </AutoSaveRow>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Toggle' }));
    expect(await screen.findByRole('status')).toHaveTextContent('Saved');
    fail = true;
    await userEvent.click(screen.getByRole('button', { name: 'Toggle' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Not allowed');
    expect(screen.queryByRole('status')).toBeNull();
  });
});
