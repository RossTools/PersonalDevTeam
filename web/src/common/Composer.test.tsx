import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Composer } from './Composer';

const box = () => screen.getByRole('textbox', { name: 'Prompt' });

describe('Composer', () => {
  it('sends on Enter, adds a line on Shift+Enter, and clears after sending', async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<Composer onSend={onSend} />);
    expect(box()).toHaveFocus();
    await userEvent.keyboard('one{Shift>}{Enter}{/Shift}two{Enter}');
    expect(onSend).toHaveBeenCalledExactlyOnceWith('one\ntwo');
    expect(box()).toHaveValue('');
  });

  it('with submitOn="mod-enter", Enter adds a line and Ctrl/Cmd+Enter sends', async () => {
    const onSend = vi.fn();
    render(<Composer onSend={onSend} submitOn="mod-enter" />);
    await userEvent.keyboard('a{Enter}b');
    expect(onSend).not.toHaveBeenCalled();
    await userEvent.keyboard('{Control>}{Enter}{/Control}');
    expect(onSend).toHaveBeenCalledWith('a\nb');
    await userEvent.keyboard('c{Meta>}{Enter}{/Meta}');
    expect(onSend).toHaveBeenLastCalledWith('c');
  });

  it('does not send blank text; Send is disabled', async () => {
    const onSend = vi.fn();
    render(<Composer onSend={onSend} />);
    await userEvent.keyboard('   {Enter}');
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  });

  it('is pending while sending and sends once', async () => {
    let resolve!: () => void;
    const onSend = vi.fn(() => new Promise<void>((r) => { resolve = r; }));
    render(<Composer onSend={onSend} />);
    await userEvent.keyboard('hi{Enter}{Enter}');
    expect(onSend).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: 'Send' })).toHaveAttribute('data-pending', 'true');
    await act(async () => { resolve(); });
    expect(box()).toHaveValue('');
  });

  it('keeps the text and shows the error when sending fails', async () => {
    const onSend = vi.fn().mockRejectedValue(new Error('Session ended'));
    render(<Composer onSend={onSend} />);
    await userEvent.keyboard('keep me{Enter}');
    expect(await screen.findByRole('alert')).toHaveTextContent('Session ended');
    expect(box()).toHaveValue('keep me');
  });

  it('Escape and Close call onCancel; no Close without it', async () => {
    const onCancel = vi.fn();
    const { unmount } = render(<Composer onSend={() => {}} onCancel={onCancel} />);
    await userEvent.keyboard('{Escape}');
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onCancel).toHaveBeenCalledTimes(2);
    unmount();
    render(<Composer onSend={() => {}} />);
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
  });

  it('sends with the Send button', async () => {
    const onSend = vi.fn();
    render(<Composer onSend={onSend} label="Comment" sendLabel="Post" />);
    await userEvent.type(screen.getByRole('textbox', { name: 'Comment' }), 'LGTM');
    await userEvent.click(screen.getByRole('button', { name: 'Post' }));
    expect(onSend).toHaveBeenCalledWith('LGTM');
  });
});
