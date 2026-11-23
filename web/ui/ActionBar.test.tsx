import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ActionBar } from './ActionBar';

describe('ActionBar', () => {
  it('has the note, the buttons, the hint and the end buttons in a named group', () => {
    render(
      <ActionBar aria-label="Your choice" note={<textarea aria-label="Note" />} hint="1 of 2 answered"
        end={<button type="button">Dismiss</button>} below={<p>Outcomes</p>}>
        <button type="button">Send answers</button>
      </ActionBar>,
    );
    const bar = screen.getByRole('group', { name: 'Your choice' });
    expect(within(bar).getByRole('textbox', { name: 'Note' })).toBeInTheDocument();
    expect(within(bar).getAllByRole('button').map((b) => b.textContent)).toEqual(['Send answers', 'Dismiss']);
    expect(bar).toHaveTextContent('1 of 2 answered');
    expect(bar).toHaveTextContent('Outcomes');
  });

  it('keeps the hint in a live region that is there before it says anything', () => {
    const { rerender } = render(<ActionBar><button type="button">Close</button></ActionBar>);
    const live = screen.getByRole('group').querySelector('[aria-live=polite]')!;
    expect(live).toBeEmptyDOMElement();
    rerender(<ActionBar hint="Press again to confirm"><button type="button">Close</button></ActionBar>);
    expect(screen.getByRole('group').querySelector('[aria-live=polite]')).toBe(live);
    expect(live).toHaveTextContent('Press again to confirm');
  });
});
