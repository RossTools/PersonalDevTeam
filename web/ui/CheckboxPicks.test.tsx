import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { CheckboxPicks } from './CheckboxPicks';

function Projects({ initial = [] as string[] }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <CheckboxPicks
        label="Projects"
        hint="Every project when none is ticked."
        options={[{ id: 'a', name: 'Alpha' }, { id: 'b', name: 'Beta' }]}
        value={value}
        onChange={setValue}
      />
      <output>{value.join(',')}</output>
    </>
  );
}

describe('CheckboxPicks', () => {
  it('is a labelled, described group whose checkboxes add and remove ids', async () => {
    render(<Projects initial={['b']} />);
    const group = screen.getByRole('group', { name: 'Projects' });
    expect(group).toHaveAccessibleDescription('Every project when none is ticked.');
    expect(screen.getByRole('checkbox', { name: 'Beta' })).toBeChecked();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Alpha' }));
    expect(screen.getByRole('status')).toHaveTextContent('b,a');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Beta' }));
    expect(screen.getByRole('status')).toHaveTextContent('a');
  });

  it('renders nothing without options', () => {
    const { container } = render(<CheckboxPicks label="Agents" options={[]} value={[]} onChange={() => {}} />);
    expect(container).toBeEmptyDOMElement();
  });
});
