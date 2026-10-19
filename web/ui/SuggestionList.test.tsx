import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SuggestionList, suggestionId } from './SuggestionList';

describe('SuggestionList', () => {
  it('marks the active option and picks enabled ones only', async () => {
    const onPick = vi.fn();
    render(
      <SuggestionList
        id="s"
        label="Commands"
        active={1}
        onPick={onPick}
        items={[{ id: 'a', label: '/a', detail: 'first' }, { id: 'b', label: '/b' }, { id: 'c', label: '/c', disabled: true, hint: 'Only in the terminal' }]}
      />,
    );
    const opts = screen.getAllByRole('option');
    expect(opts[1]).toHaveAttribute('aria-selected', 'true');
    expect(opts[1].id).toBe(suggestionId('s', 1));
    await userEvent.click(opts[2]);
    expect(onPick).not.toHaveBeenCalled();
    await userEvent.click(opts[0]);
    expect(onPick).toHaveBeenCalledWith(0);
  });

  it('says when nothing matches', () => {
    render(<SuggestionList id="s" label="Files" active={0} onPick={() => {}} items={[]} empty="No matching files" />);
    expect(screen.getByText('No matching files')).toBeInTheDocument();
  });
});
