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
});
