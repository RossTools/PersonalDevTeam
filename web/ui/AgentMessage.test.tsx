import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { AgentMessage, Avatar, PassList } from './AgentMessage';

describe('AgentMessage', () => {
  it('shows who said it, with their avatar, over the text', () => {
    render(<AgentMessage name="Coder" label="Coder's context">The receipt email is in.</AgentMessage>);
    const message = screen.getByRole('group', { name: 'From Coder' });
    expect(message).toHaveTextContent("Coder's context");
    expect(message).toHaveTextContent('The receipt email is in.');
    expect(message.querySelector('[aria-hidden]')).toHaveTextContent('C');
  });

  it('gives a name the same colour every time', () => {
    const { container } = render(<><Avatar name="QA" /><Avatar name="QA" /><Avatar name="reviewer" /></>);
    const [a, b, c] = [...container.querySelectorAll('span')];
    expect(a.dataset.series).toBe(b.dataset.series);
    expect(c).toHaveTextContent('R');
  });
});

describe('PassList', () => {
  it('lists each pass with its persona, its outcome as a chip and its summary', () => {
    render(<PassList passes={[
      { name: 'Reviewer', title: 'Pass 1 · Reviewer', outcome: 'changes_requested', tone: 'bad', summary: 'Rates are hard-coded.' },
      { name: 'QA', outcome: null },
    ]} />);
    const rows = within(screen.getByRole('list', { name: 'Passes' })).getAllByRole('listitem');
    expect(rows[0]).toHaveTextContent('Pass 1 · Reviewer');
    expect(within(rows[0]).getByText('changes_requested').className).toContain('text-failed');
    expect(rows[0]).toHaveTextContent('Rates are hard-coded.');
    expect(rows[1]).toHaveTextContent('no outcome');
  });
});
