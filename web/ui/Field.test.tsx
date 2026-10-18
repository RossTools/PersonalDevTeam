import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { Field } from './Field';
import { Input } from './Input';
import { Select } from './Select';
import { Switch } from './Switch';
import { Textarea } from './Textarea';

describe('Field', () => {
  it('labels the control and describes it with the hint', async () => {
    render(
      <Field label="Owner" hint="Tailscale login of the owner">
        <Input />
      </Field>,
    );
    const input = screen.getByRole('textbox', { name: 'Owner' });
    expect(input).toHaveAccessibleDescription('Tailscale login of the owner');
    expect(input).not.toHaveAttribute('aria-invalid');
    await userEvent.click(screen.getByText('Owner'));
    expect(input).toHaveFocus();
  });

  it('shows the error, marks the control invalid and describes it with hint + error', () => {
    render(
      <Field label="Key" hint="2–6 letters" error="Key is taken">
        <Input />
      </Field>,
    );
    const input = screen.getByRole('textbox', { name: 'Key' });
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription('2–6 letters Key is taken');
    expect(screen.getByRole('alert')).toHaveTextContent('Key is taken');
  });

  it('keeps an id and description the control already has', () => {
    render(
      <>
        <p id="extra">Extra</p>
        <Field label="Notes" hint="Optional">
          <Textarea id="notes" aria-describedby="extra" />
        </Field>
      </>,
    );
    const t = screen.getByRole('textbox', { name: 'Notes' });
    expect(t).toHaveAttribute('id', 'notes');
    expect(t).toHaveAccessibleDescription('Extra Optional');
  });

  it('labels a Select and a Switch through aria-labelledby', () => {
    render(
      <>
        <Field label="Agent">
          <Select value="claude" onChange={() => {}} options={[{ value: 'claude', label: 'Claude' }]} />
        </Field>
        <Field label="Remote access" inline hint="Over Tailscale">
          <Switch />
        </Field>
      </>,
    );
    expect(screen.getByRole('button', { name: /Agent/ })).toBeInTheDocument();
    const sw = screen.getByRole('switch', { name: 'Remote access' });
    expect(sw).toHaveAccessibleDescription('Over Tailscale');
  });

  it('several labelled controls in one Field keep their names and distinct ids, and share the error', () => {
    render(
      <Field label="Run ports" error="End must be after start">
        <div>
          <Input aria-label="Start" />
          <Input aria-label="End" />
        </div>
      </Field>,
    );
    const start = screen.getByRole('textbox', { name: 'Start' });
    const end = screen.getByRole('textbox', { name: 'End' });
    expect(start.id && start.id === end.id).toBeFalsy();
    expect(end).toHaveAccessibleDescription('End must be after start');
    expect(end).toHaveAttribute('aria-invalid', 'true');
  });

  it('inline lays the row out side by side from sm up', () => {
    const { container } = render(
      <Field label="Theme" inline>
        <Input />
      </Field>,
    );
    expect((container.firstChild as HTMLElement).className).toContain('sm:flex-row');
  });

  it('disabled disables the control', () => {
    render(
      <Field label="Port" disabled>
        <Input />
      </Field>,
    );
    expect(screen.getByRole('textbox', { name: 'Port' })).toBeDisabled();
  });
});
