import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { ConfirmProvider, useConfirm, type ConfirmOptions, type ConfirmResult } from './Confirm';

function Harness({ options }: { options: ConfirmOptions }) {
  const confirm = useConfirm();
  const [result, setResult] = useState<ConfirmResult | null>(null);
  return (
    <>
      <button onClick={async () => setResult(await confirm(options))}>Remove</button>
      <output aria-label="result">{result ? JSON.stringify(result) : ''}</output>
    </>
  );
}

const remove: ConfirmOptions = {
  title: 'Remove worktree feature-x?',
  body: 'Its uncommitted changes are lost.',
  action: 'Remove worktree',
  danger: true,
  checkbox: { label: 'Also delete the branch' },
};

function setup(options: ConfirmOptions = remove) {
  render(
    <ConfirmProvider>
      <Harness options={options} />
    </ConfirmProvider>,
  );
  return screen.getByRole('button', { name: 'Remove' });
}

describe('useConfirm', () => {
  it('shows title, consequence and a named danger action', async () => {
    await userEvent.click(setup());
    const d = await screen.findByRole('alertdialog', { name: 'Remove worktree feature-x?' });
    expect(d).toHaveAccessibleDescription('Its uncommitted changes are lost.');
    expect(screen.getByRole('button', { name: 'Remove worktree' })).toBeInTheDocument();
    // Destructive confirms start on Cancel.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus());
  });

  it('resolves ok with the checkbox state', async () => {
    await userEvent.click(setup());
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Also delete the branch' }));
    await userEvent.click(screen.getByRole('button', { name: 'Remove worktree' }));
    await waitFor(() => expect(screen.getByLabelText('result')).toHaveTextContent('{"ok":true,"checked":true}'));
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });

  it('defaultChecked starts checked', async () => {
    await userEvent.click(setup({ ...remove, checkbox: { label: 'Also delete the branch', defaultChecked: true } }));
    expect(await screen.findByRole('checkbox', { name: 'Also delete the branch' })).toBeChecked();
    await userEvent.click(screen.getByRole('button', { name: 'Remove worktree' }));
    await waitFor(() => expect(screen.getByLabelText('result')).toHaveTextContent('{"ok":true,"checked":true}'));
  });

  it('Cancel resolves ok:false', async () => {
    await userEvent.click(setup());
    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.getByLabelText('result')).toHaveTextContent('{"ok":false,"checked":false}'));
  });

  it('Escape resolves ok:false and focus returns to the opener', async () => {
    const opener = setup();
    await userEvent.click(opener);
    await screen.findByRole('alertdialog');
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.getByLabelText('result')).toHaveTextContent('"ok":false'));
    expect(screen.queryByRole('alertdialog')).toBeNull();
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it('non-danger confirms use the primary action', async () => {
    await userEvent.click(setup({ title: 'Restart the agent?', action: 'Restart' }));
    const b = await screen.findByRole('button', { name: 'Restart' });
    expect(b.className).toContain('bg-primary');
    expect(screen.queryByRole('checkbox')).toBeNull();
  });

  it('throws without a provider', () => {
    function Bare() {
      useConfirm();
      return null;
    }
    const err = console.error;
    console.error = () => {};
    try {
      expect(() => render(<Bare />)).toThrow(/ConfirmProvider/);
    } finally {
      console.error = err;
    }
  });
});
