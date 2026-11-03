import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Disclosure } from './Disclosure';

describe('Disclosure', () => {
  it('starts closed: the trigger says so and the content is not rendered', () => {
    render(<Disclosure title="Advanced">The settings</Disclosure>);
    expect(screen.getByRole('button', { name: 'Advanced' })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('The settings')).toBeNull();
  });

  it('opens and closes on click, and from the keyboard', async () => {
    render(<Disclosure title="Thinking">The thoughts</Disclosure>);
    const trigger = screen.getByRole('button', { name: 'Thinking' });
    await userEvent.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('The thoughts')).toBeVisible();
    expect(trigger).toHaveAttribute('aria-controls', screen.getByText('The thoughts').closest('[id]')!.id);
    await userEvent.keyboard('{Enter}');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await userEvent.keyboard(' ');
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
  });

  it('can start open, or be controlled', async () => {
    const onOpenChange = vi.fn();
    const { rerender } = render(<Disclosure title="Input" defaultOpen>JSON</Disclosure>);
    expect(screen.getByText('JSON')).toBeVisible();
    rerender(<Disclosure title="Steps" open={false} onOpenChange={onOpenChange}>Rows</Disclosure>);
    await userEvent.click(screen.getByRole('button', { name: 'Steps' }));
    expect(onOpenChange).toHaveBeenCalledWith(true);
    expect(screen.queryByText('Rows')).toBeNull();
  });

  it('a row shows its summary and trailing content in the trigger', () => {
    render(<Disclosure variant="row" title="Details" summary="Ready · High" trailing={<span>3</span>}>Body</Disclosure>);
    const trigger = screen.getByRole('button', { name: /Details/ });
    expect(trigger).toHaveTextContent('Ready · High');
    expect(trigger).toHaveTextContent('3');
    expect(trigger.className).toContain('pointer-coarse:min-h-control-touch');
  });

  it('aria-label names the trigger and the panel when the trigger shows more', async () => {
    render(<Disclosure variant="row" title="Alpha" trailing={<span>ALP</span>} aria-label="Alpha">Sections</Disclosure>);
    await userEvent.click(screen.getByRole('button', { name: 'Alpha' }));
    expect(screen.getByRole('group', { name: 'Alpha' })).toHaveTextContent('Sections');
  });

  it('a card: leading, title over its summary, chevron last; what is visible shows closed, the body only open', async () => {
    render(
      <Disclosure variant="card" title="Agents" summary="Found Claude Code." leading={<span>○</span>} trailing={<span>Optional</span>}
        visible={<p>git is missing</p>}>
        The table
      </Disclosure>,
    );
    const trigger = screen.getByRole('button', { name: /Agents/ });
    expect(trigger).toHaveTextContent('Found Claude Code.');
    expect(trigger).toHaveTextContent('Optional');
    expect(trigger.lastElementChild?.tagName.toLowerCase()).toBe('svg');
    expect(screen.getByText('git is missing')).toBeVisible();
    expect(screen.queryByText('The table')).toBeNull();
    await userEvent.click(trigger);
    expect(screen.getByText('The table')).toBeVisible();
    expect(screen.getByText('git is missing')).toBeVisible();
  });
});
