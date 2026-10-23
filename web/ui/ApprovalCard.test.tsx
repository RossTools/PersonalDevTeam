import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ApprovalCard } from './ApprovalCard';

describe('ApprovalCard', () => {
  it('shows the request and approves', async () => {
    const onAnswer = vi.fn();
    render(<ApprovalCard title="Run this command?" subject="cargo test" description="Runs the tests" onAnswer={onAnswer} />);
    expect(screen.getByRole('region', { name: 'Run this command?' })).toHaveTextContent('cargo test');
    expect(screen.getByText('Runs the tests')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Always allow/ })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Allow' }));
    expect(onAnswer).toHaveBeenCalledWith({ type: 'approve', always: false });
  });

  it('offers Always allow only when it can', async () => {
    const onAnswer = vi.fn();
    render(<ApprovalCard title="Edit this file?" subject="a.ts" canAlways onAnswer={onAnswer} />);
    await userEvent.click(screen.getByRole('button', { name: 'Always allow in this session' }));
    expect(onAnswer).toHaveBeenCalledWith({ type: 'approve', always: true });
  });

  it('denies with an optional reason', async () => {
    const onAnswer = vi.fn();
    render(<ApprovalCard title="Run this command?" subject="rm -rf /" onAnswer={onAnswer} />);
    await userEvent.click(screen.getByRole('button', { name: 'Deny' }));
    const why = screen.getByRole('textbox', { name: 'Tell it why' });
    await waitFor(() => expect(why).toHaveFocus());
    await userEvent.type(why, 'too dangerous{Enter}');
    expect(onAnswer).toHaveBeenCalledWith({ type: 'deny', message: 'too dangerous' });
  });

  it('y approves and n opens the denial, from outside text fields, only with keyboard on', async () => {
    const onAnswer = vi.fn();
    const { rerender } = render(<ApprovalCard title="Run this command?" subject="ls" onAnswer={onAnswer} />);
    await userEvent.keyboard('y');
    expect(onAnswer).not.toHaveBeenCalled();
    rerender(<><input aria-label="Other" /><ApprovalCard title="Run this command?" subject="ls" keyboard onAnswer={onAnswer} /></>);
    await userEvent.click(screen.getByRole('textbox', { name: 'Other' }));
    await userEvent.keyboard('y');
    expect(onAnswer).not.toHaveBeenCalled();
    await userEvent.click(document.body);
    await userEvent.keyboard('y');
    expect(onAnswer).toHaveBeenCalledWith({ type: 'approve', always: false });
    await userEvent.click(document.body);
    await userEvent.keyboard('n');
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Tell it why' })).toHaveFocus());
    await userEvent.keyboard('{Enter}');
    expect(onAnswer).toHaveBeenLastCalledWith({ type: 'deny' });
  });

  it('Escape in the reason goes back to the actions', async () => {
    render(<ApprovalCard title="Run this command?" onAnswer={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Deny' }));
    await userEvent.keyboard('{Escape}');
    expect(screen.getByRole('button', { name: 'Allow' })).toBeInTheDocument();
  });

  it('answers questions with the picked option labels', async () => {
    const onAnswer = vi.fn();
    render(
      <ApprovalCard
        kind="question"
        title="Claude asks"
        questions={[
          { question: 'Which database?', header: 'DB', options: [{ label: 'SQLite' }, { label: 'Postgres' }] },
          { question: 'Which extras?', options: [{ label: 'FTS' }, { label: 'JSON' }], multiSelect: true },
        ]}
        onAnswer={onAnswer}
      />,
    );
    const submit = screen.getByRole('button', { name: 'Submit' });
    expect(submit).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'SQLite' }));
    await userEvent.click(screen.getByRole('button', { name: 'Postgres' }));
    expect(screen.getByRole('button', { name: 'SQLite' })).toHaveAttribute('aria-pressed', 'false');
    await userEvent.click(screen.getByRole('button', { name: 'FTS' }));
    await userEvent.click(screen.getByRole('button', { name: 'JSON' }));
    await userEvent.click(submit);
    expect(onAnswer).toHaveBeenCalledWith({ type: 'question', answers: { 'Which database?': ['Postgres'], 'Which extras?': ['FTS', 'JSON'] } });
  });

  it('takes an answer in the user’s own words', async () => {
    const onAnswer = vi.fn();
    render(
      <ApprovalCard
        kind="question"
        title="Claude asks"
        questions={[{ question: 'Which word?', options: [{ label: 'apple', description: 'A fruit' }, { label: 'pear' }] }]}
        onAnswer={onAnswer}
      />,
    );
    expect(screen.getByText('A fruit')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'apple' }));
    await userEvent.click(screen.getByRole('button', { name: 'Other…' }));
    expect(screen.getByRole('button', { name: 'apple' })).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByRole('button', { name: 'Submit' })).toBeDisabled();
    await userEvent.type(screen.getByRole('textbox', { name: 'Your answer' }), ' plum {Enter}');
    expect(onAnswer).toHaveBeenCalledWith({ type: 'question', answers: { 'Which word?': ['plum'] } });
  });

  it('approves a plan into Accept edits when offered', async () => {
    const onAnswer = vi.fn();
    render(<ApprovalCard kind="plan" title="Approve this plan?" plan={<p>1. Do it</p>} planAcceptEdits onAnswer={onAnswer} />);
    await userEvent.click(screen.getByRole('button', { name: 'Approve, accept edits' }));
    expect(onAnswer).toHaveBeenLastCalledWith({ type: 'plan', approve: true, mode: 'accept_edits' });
    await userEvent.click(screen.getByRole('button', { name: 'Approve, ask before edits' }));
    expect(onAnswer).toHaveBeenLastCalledWith({ type: 'plan', approve: true });
  });

  it('shows a preview of the change', () => {
    render(<ApprovalCard title="Edit this file?" subject="a.rs" preview={<p>the diff</p>} onAnswer={() => {}} />);
    expect(screen.getByRole('region', { name: 'Edit this file?' })).toHaveTextContent('the diff');
  });

  it('approves a plan or keeps planning with feedback', async () => {
    const onAnswer = vi.fn();
    render(<ApprovalCard kind="plan" title="Approve this plan?" plan={<p>1. Do it</p>} onAnswer={onAnswer} />);
    expect(screen.getByText('1. Do it')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Keep planning' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'What to change' }), 'smaller steps');
    await userEvent.click(screen.getByRole('button', { name: 'Keep planning' }));
    expect(onAnswer).toHaveBeenCalledWith({ type: 'plan', approve: false, feedback: 'smaller steps' });
  });

  it('shows the answer pending and ignores more presses until it settles', async () => {
    let resolve!: () => void;
    const onAnswer = vi.fn(() => new Promise<void>((r) => { resolve = r; }));
    render(<ApprovalCard title="Run this command?" onAnswer={onAnswer} />);
    await userEvent.click(screen.getByRole('button', { name: 'Allow' }));
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
    resolve();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Deny' })).toBeEnabled());
    expect(onAnswer).toHaveBeenCalledTimes(1);
  });
});
