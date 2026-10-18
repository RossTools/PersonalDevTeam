import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Button } from './Button';

describe('Button', () => {
  it('maps variants onto token classes', () => {
    render(
      <>
        <Button variant="primary">P</Button>
        <Button>S</Button>
        <Button variant="ghost">G</Button>
        <Button variant="danger">D</Button>
      </>,
    );
    expect(screen.getByRole('button', { name: 'P' }).className).toContain('bg-primary');
    expect(screen.getByRole('button', { name: 'S' }).className).toContain('border-border');
    expect(screen.getByRole('button', { name: 'G' }).className).toContain('text-muted-foreground');
    expect(screen.getByRole('button', { name: 'D' }).className).toContain('text-destructive');
  });

  it('sizes: md is the default, sm is 24px, both grow on touch', () => {
    render(
      <>
        <Button>Md</Button>
        <Button size="sm">Sm</Button>
      </>,
    );
    const md = screen.getByRole('button', { name: 'Md' });
    expect(md.className).toContain('h-control-md');
    expect(md.className).toContain('pointer-coarse:h-control-touch');
    expect(screen.getByRole('button', { name: 'Sm' }).className).toContain('h-control-sm');
  });

  it('shows title as a tooltip on keyboard focus', async () => {
    render(<Button title="Opens the log">Log</Button>);
    await userEvent.tab();
    expect(screen.getByRole('button', { name: 'Log' })).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Opens the log');
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('defaults to type=button and forwards aria-* and className (last wins)', () => {
    render(
      <Button aria-expanded={false} aria-pressed className="px-7">
        X
      </Button>,
    );
    const b = screen.getByRole('button', { name: 'X' });
    expect(b).toHaveAttribute('type', 'button');
    expect(b).toHaveAttribute('aria-expanded', 'false');
    expect(b).toHaveAttribute('aria-pressed', 'true');
    expect(b.className).toContain('px-7');
    expect(b.className).not.toContain('px-2.5');
  });

  it('onClick fires on fireEvent.click, userEvent.click and the keyboard', async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Go</Button>);
    const b = screen.getByRole('button', { name: 'Go' });
    fireEvent.click(b);
    expect(onClick).toHaveBeenCalledTimes(1);
    await userEvent.click(b);
    expect(onClick).toHaveBeenCalledTimes(2);
    b.focus();
    await userEvent.keyboard('{Enter}');
    await userEvent.keyboard(' ');
    expect(onClick).toHaveBeenCalledTimes(4);
  });

  it('disabled: native disabled, no onClick', async () => {
    const onClick = vi.fn();
    render(
      <Button disabled onClick={onClick}>
        No
      </Button>,
    );
    const b = screen.getByRole('button', { name: 'No' });
    expect(b).toBeDisabled();
    fireEvent.click(b);
    await userEvent.click(b);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('type=submit submits its form (fireEvent and userEvent)', async () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button type="submit">Save</Button>
      </form>,
    );
    const b = screen.getByRole('button', { name: 'Save' });
    expect(b).toHaveAttribute('type', 'submit');
    fireEvent.click(b);
    expect(onSubmit).toHaveBeenCalledTimes(1);
    await userEvent.click(b);
    expect(onSubmit).toHaveBeenCalledTimes(2);
  });

  it('pending shows a spinner, keeps the label (for width and name) and ignores presses', async () => {
    const onClick = vi.fn();
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button type="submit" pending onClick={onClick}>
          Saving
        </Button>
      </form>,
    );
    const b = screen.getByRole('button', { name: 'Saving' });
    expect(b).toHaveAttribute('aria-disabled', 'true');
    expect(b).toHaveAttribute('data-pending', 'true');
    expect(b.querySelector('[data-slot=spinner]')).not.toBeNull();
    expect(screen.getByText('Saving')).toHaveClass('invisible');
    await userEvent.click(b);
    fireEvent.click(b);
    expect(onClick).not.toHaveBeenCalled();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('renders a leading icon from children', () => {
    render(
      <Button>
        <svg data-testid="icon" />
        New
      </Button>,
    );
    expect(screen.getByRole('button', { name: 'New' })).toContainElement(screen.getByTestId('icon'));
  });
});
