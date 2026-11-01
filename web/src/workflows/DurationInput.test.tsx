import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { renderWithQuery } from '../test-utils';
import { ParamsForm } from './NewWorkflowDialog';

const wait = { id: 'mergeWait', title: 'Merge wait', description: '', type: 'duration' as const, required: false };

/** The form holding its values, telling `onChange` each time they change. */
function Form({ start, onChange }: { start: number; onChange: (v: Record<string, unknown>) => void }) {
  const [values, setValues] = useState<Record<string, unknown>>({ mergeWait: start });
  return <ParamsForm params={[wait]} values={values} onChange={(v) => { setValues(v); onChange(v); }} />;
}

describe('a duration param', () => {
  it('shows milliseconds in the largest whole unit and gives back milliseconds', async () => {
    const onChange = vi.fn();
    renderWithQuery(<Form start={3 * 86_400_000} onChange={onChange} />);
    const amount = screen.getByLabelText('Merge wait') as HTMLInputElement;
    expect(amount.value).toBe('3');
    expect(screen.getByRole('button', { name: /Merge wait unit/ }).textContent).toContain('days');
    await userEvent.clear(amount);
    await userEvent.type(amount, '2');
    expect(onChange).toHaveBeenLastCalledWith({ mergeWait: 2 * 86_400_000 });
  });

  it('keeps the amount when the unit changes', async () => {
    const onChange = vi.fn();
    renderWithQuery(<Form start={6 * 3_600_000} onChange={onChange} />);
    expect((screen.getByLabelText('Merge wait') as HTMLInputElement).value).toBe('6');
    await userEvent.click(screen.getByRole('button', { name: /Merge wait unit/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'minutes' }));
    expect(onChange).toHaveBeenLastCalledWith({ mergeWait: 6 * 60_000 });
    expect((screen.getByLabelText('Merge wait') as HTMLInputElement).value).toBe('6');
  });
});
