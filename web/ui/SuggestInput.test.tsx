import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SuggestInput } from './SuggestInput';

const field = () => screen.getByRole('combobox', { name: 'Add label' });

describe('SuggestInput', () => {
  it('commits a suggestion on one click and clears', async () => {
    const onCommit = vi.fn();
    render(<SuggestInput aria-label="Add label" suggestions={['backend', 'ui']} onCommit={onCommit} />);
    await userEvent.click(field());
    await userEvent.click(await screen.findByRole('option', { name: 'ui' }));
    expect(onCommit).toHaveBeenCalledWith('ui');
    expect(field()).toHaveValue('');
  });

  it('filters suggestions by the typed text and picks the highlighted one with Enter', async () => {
    const onCommit = vi.fn();
    render(<SuggestInput aria-label="Add label" suggestions={['backend', 'ui']} onCommit={onCommit} />);
    await userEvent.type(field(), 'back');
    expect(screen.queryByRole('option', { name: 'ui' })).not.toBeInTheDocument();
    await userEvent.keyboard('{ArrowDown}{Enter}');
    expect(onCommit).toHaveBeenCalledWith('backend');
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('commits new text with Enter or a commit key', async () => {
    const onCommit = vi.fn();
    render(<SuggestInput aria-label="Add label" suggestions={['backend']} onCommit={onCommit} commitKeys={[',']} />);
    await userEvent.type(field(), 'perf{Enter}');
    expect(onCommit).toHaveBeenLastCalledWith('perf');
    expect(field()).toHaveValue('');
    await userEvent.type(field(), 'docs,');
    expect(onCommit).toHaveBeenLastCalledWith('docs');
    await userEvent.type(field(), '   {Enter}');
    expect(onCommit).toHaveBeenCalledTimes(2);
  });
});
