import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StageStepper } from './StageStepper';

describe('StageStepper', () => {
  it('lists the stages and marks the current one', () => {
    render(<StageStepper stage="ready" />);
    const steps = within(screen.getByRole('list', { name: 'Stage' })).getAllByRole('listitem');
    expect(steps.map((s) => s.textContent)).toEqual(['Draft', 'Ready', 'To do', 'In progress', 'Done']);
    expect(steps[1]).toHaveAttribute('aria-current', 'step');
    expect(steps[0]).toHaveAttribute('data-state', 'complete');
    expect(steps[2]).toHaveAttribute('data-state', 'upcoming');
    expect(steps[0]).not.toHaveAttribute('aria-current');
  });

  it("shows Won't do as a crossed final step", () => {
    render(<StageStepper stage="wont_do" />);
    const steps = screen.getAllByRole('listitem');
    expect(steps).toHaveLength(5);
    expect(steps[4]).toHaveTextContent("Won't do");
    expect(steps[4]).toHaveAttribute('aria-current', 'step');
    expect(within(steps[4]).getByText("Won't do").className).toContain('line-through');
    expect(screen.queryByText('Done')).toBeNull();
  });

  it('compact keeps every label for screen readers but shows only the current one', () => {
    render(<StageStepper stage="in_progress" compact />);
    expect(screen.getByText('Draft').className).toContain('sr-only');
    expect(screen.getByText('In progress').className).not.toContain('sr-only');
  });
});
