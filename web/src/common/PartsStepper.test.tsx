import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PARTS_BAR_ABOVE, PartsStepper, type PartState, type PartsProgress } from './PartsStepper';

const progressOf = (segments: PartState[]): PartsProgress => {
  const n = (s: PartState) => segments.filter((x) => x === s).length;
  return { total: segments.length, done: n('done'), merged: n('merged'), review: n('review'), progress: n('progress'), todo: n('todo'), backlog: n('backlog'), segments };
};
const many = (counts: Partial<Record<PartState, number>>) =>
  progressOf(Object.entries(counts).flatMap(([s, k]) => Array<PartState>(k).fill(s as PartState)));

describe('PartsStepper', () => {
  it('draws a dot per part, in order, up to the threshold', () => {
    const { container } = render(<PartsStepper parts={many({ done: 15, progress: PARTS_BAR_ABOVE - 15 })} showCount />);
    const dots = container.querySelectorAll('[data-state]');
    expect(dots).toHaveLength(PARTS_BAR_ABOVE);
    expect(container.querySelector('[data-bar]')).toBeNull();
    expect(screen.getByText(`15/${PARTS_BAR_ABOVE}`)).toBeInTheDocument();
  });

  it('above the threshold, draws one bar with a segment per state, sized by its count, and the counts in the tooltip', () => {
    const { container } = render(<PartsStepper parts={many({ backlog: 30, done: 15, review: 2, progress: 4, todo: 19 })} showCount />);
    const bar = container.querySelector('[data-bar]')!;
    const segments = [...bar.querySelectorAll<HTMLElement>('[data-state]')];
    expect(segments.map((s) => s.dataset.state)).toEqual(['done', 'review', 'progress', 'todo', 'backlog']);
    expect(segments.map((s) => s.style.flexGrow)).toEqual(['15', '2', '4', '19', '30']);
    const img = screen.getByRole('img', { name: '15 of 70 parts done, 2 in review, 4 in progress, 19 to do, 30 not started' });
    expect(img).toHaveAttribute('title', '15 of 70 parts done, 2 in review, 4 in progress, 19 to do, 30 not started');
    expect(screen.getByText('15/70')).toBeInTheDocument();
  });
});
