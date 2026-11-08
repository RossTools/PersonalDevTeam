import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { answerText, NO_ANSWER, QuestionCard, type QuestionAnswer } from './QuestionCard';

function Card({ choices = ['Attach a PDF', 'Text is enough'], numbers = true }: { choices?: string[]; numbers?: boolean }) {
  const [value, setValue] = useState<QuestionAnswer>(NO_ANSWER);
  return (
    <>
      <QuestionCard id="q" index={0} total={2} question="Attach a PDF?" label="Attach a PDF?" numbers={numbers}
        choices={choices.map((label) => ({ label, description: label === 'Attach a PDF' ? 'About a day more.' : undefined }))}
        value={value} onChange={setValue} />
      <output>{answerText(value)}</output>
    </>
  );
}

function Several({ onSubmit }: { onSubmit?: () => void }) {
  const [value, setValue] = useState<QuestionAnswer>(NO_ANSWER);
  return (
    <>
      <QuestionCard id="m" index={0} total={1} label="What to cache?" multiple onSubmit={onSubmit}
        choices={[{ label: 'Tickets' }, { label: 'Runs' }]} value={value} onChange={setValue} />
      <output>{[...(value.choices ?? []), ...(value.other ? [value.other] : [])].join(', ')}</output>
    </>
  );
}

describe('QuestionCard', () => {
  it('with multiple, ticks several choices and adds own words to them; Enter there submits', async () => {
    const onSubmit = vi.fn();
    render(<Several onSubmit={onSubmit} />);
    expect(screen.getByRole('group', { name: 'What to cache?' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Tickets' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Runs' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Something else' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Something else for question 1' }), 'Notes{Enter}');
    expect(screen.getByRole('status')).toHaveTextContent('Tickets, Runs, Notes');
    expect(onSubmit).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Tickets' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Something else' }));
    expect(screen.getByRole('status')).toHaveTextContent(/^Runs$/);
  });

  it('shows "1 / 2", the choices as radio options with their keys and descriptions, and Something else', () => {
    render(<Card />);
    const card = screen.getByRole('region', { name: 'Question 1 of 2' });
    expect(card).toHaveTextContent('1 / 2');
    expect(screen.getByRole('radiogroup', { name: 'Attach a PDF?' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /Attach a PDF/ })).toHaveAccessibleDescription('About a day more.');
    expect(screen.getByRole('radio', { name: /Text is enough/ })).toHaveTextContent('2');
    expect(screen.getByRole('radio', { name: 'Something else' })).toBeInTheDocument();
  });

  it('picks one choice, or the user\'s own words', async () => {
    render(<Card />);
    await userEvent.click(screen.getByRole('radio', { name: /Text is enough/ }));
    expect(screen.getByRole('radio', { name: /Text is enough/ })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('status')).toHaveTextContent('Text is enough');
    await userEvent.click(screen.getByRole('radio', { name: 'Something else' }));
    expect(screen.getByRole('textbox', { name: 'Something else for question 1' })).toHaveFocus();
    await userEvent.keyboard('Later');
    expect(screen.getByRole('radio', { name: /Text is enough/ })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('status')).toHaveTextContent('Later');
  });

  it('moves between the options with the arrow keys', async () => {
    render(<Card />);
    screen.getByRole('radio', { name: /Attach a PDF/ }).focus();
    await userEvent.keyboard('{ArrowDown}');
    expect(screen.getByRole('radio', { name: /Text is enough/ })).toHaveFocus();
    expect(screen.getByRole('radio', { name: /Text is enough/ })).toHaveAttribute('aria-checked', 'true');
  });

  it('is a text box when the question has no choices, and hides the keys without numbers', () => {
    render(<Card choices={[]} numbers={false} />);
    expect(screen.queryByRole('radiogroup')).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Answer 1' })).toBeInTheDocument();
  });
});
