import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { InlineRename } from './InlineRename';

function setup(initial = 'Claude A') {
  const onCommit = vi.fn();
  const onCancel = vi.fn();
  render(<><InlineRename label="Tab name" initial={initial} onCommit={onCommit} onCancel={onCancel} maxLength={20} /><button type="button">elsewhere</button></>);
  return { onCommit, onCancel, input: screen.getByRole('textbox', { name: 'Tab name' }) as HTMLInputElement };
}

describe('InlineRename', () => {
  it('is focused with its text selected, and limits the length', () => {
    const { input } = setup();
    expect(input).toHaveFocus();
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 'Claude A'.length]);
    expect(input).toHaveAttribute('maxLength', '20');
  });

  it('commits the trimmed name on Enter, once', async () => {
    const { onCommit, onCancel } = setup();
    await userEvent.keyboard('  Reviewer  {Enter}');
    await userEvent.click(screen.getByRole('button', { name: 'elsewhere' }));
    expect(onCommit).toHaveBeenCalledExactlyOnceWith('Reviewer');
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('commits on blur', async () => {
    const { onCommit } = setup();
    await userEvent.keyboard('Builder');
    await userEvent.click(screen.getByRole('button', { name: 'elsewhere' }));
    expect(onCommit).toHaveBeenCalledWith('Builder');
  });

  it('cancels on Escape', async () => {
    const { onCommit, onCancel } = setup();
    await userEvent.keyboard('Builder{Escape}');
    expect(onCommit).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it('ignores an unchanged or empty name', async () => {
    const a = setup();
    await userEvent.keyboard('{Enter}');
    expect(a.onCancel).toHaveBeenCalledOnce();
    expect(a.onCommit).not.toHaveBeenCalled();
  });

  it('ignores a blank new name', async () => {
    const { onCommit, onCancel } = setup('');
    await userEvent.keyboard('   {Enter}');
    expect(onCommit).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
