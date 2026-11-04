import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { stripAnsi, ToolCallRow, ToolOutput } from './ToolCallRow';

describe('ToolCallRow', () => {
  it('shows the verb, subject and status, and expands to its details', async () => {
    render(<ToolCallRow verb="Run" subject="cargo test" status="done" meta="12 s" details={<ToolOutput label="Output">ok</ToolOutput>} />);
    const row = screen.getByRole('button', { name: /Run cargo test/ });
    expect(screen.getByRole('img', { name: 'Done' })).toBeInTheDocument();
    expect(row).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('ok')).not.toBeInTheDocument();
    await userEvent.click(row);
    expect(row).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('ok')).toBeInTheDocument();
  });

  it('puts its tags (links) beside its button, not in it', async () => {
    render(
      <ToolCallRow verb="Read" tags={<a href="/tickets/ALP-1">ALP-1</a>}>
        <ToolCallRow verb="Run" subject="ls" status="done" />
      </ToolCallRow>,
    );
    const row = screen.getByRole('button', { name: 'Read' });
    const link = screen.getByRole('link', { name: 'ALP-1' });
    expect(row).not.toContainElement(link);
    await userEvent.click(row);
    expect(screen.getByText('ls')).toBeInTheDocument();
  });

  it('names its button with `label`, and stretches it over the row with tags (PDT-548)', () => {
    render(
      <ToolCallRow verb="Read" label="Read ALP-1, ALP-2" tags={<a href="/tickets/ALP-1">ALP-1</a>}>
        <ToolCallRow verb="Run" subject="ls" status="done" />
      </ToolCallRow>,
    );
    const row = screen.getByRole('button', { name: 'Read ALP-1, ALP-2' });
    expect(row).toHaveClass('before:absolute', 'before:inset-0');
    expect(row.parentElement).toHaveClass('relative');
  });

  it('is not a button with nothing to show; running shows a spinner', () => {
    render(<ToolCallRow verb="Read" subject="a.ts" status="running" />);
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(screen.getByRole('status', { name: 'Running' })).toBeInTheDocument();
  });

  it('nests children (a summary of calls)', async () => {
    render(
      <ToolCallRow verb="Ran 2 commands">
        <ToolCallRow verb="Run" subject="ls" status="failed" />
        <ToolCallRow verb="Run" subject="pwd" status="declined" />
      </ToolCallRow>,
    );
    await userEvent.click(screen.getByRole('button', { name: /Ran 2 commands/ }));
    expect(screen.getByRole('img', { name: 'Failed' })).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Declined' })).toBeInTheDocument();
  });

  it('strips terminal escapes from output', () => {
    expect(stripAnsi('\u001b[31mred\u001b[0m \u001b]8;;http://x\u0007link\u001b]8;;\u0007')).toBe('red link');
  });
});
