import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { DiffViewer, diffStats, parseDiff } from './DiffViewer';

const TWO_FILES = [
  '--- a/src/a.rs',
  '+++ b/src/a.rs',
  '@@ -10,3 +10,4 @@',
  ' keep',
  '-old',
  '+new',
  '+more',
  ' tail',
  '--- /dev/null',
  '+++ b/notes.txt',
  '@@ -0,0 +1,1 @@',
  '+hello',
  '',
].join('\n');

describe('parseDiff', () => {
  it('reads files, hunks and line numbers', () => {
    const files = parseDiff(TWO_FILES);
    expect(files.map((f) => [f.path, f.created, f.added, f.removed])).toEqual([
      ['src/a.rs', false, 2, 1],
      ['notes.txt', true, 1, 0],
    ]);
    const lines = files[0].hunks[0].lines;
    expect(lines.map((l) => [l.kind, l.old, l.new])).toEqual([
      ['ctx', 10, 10],
      ['del', 11, undefined],
      ['add', undefined, 11],
      ['add', undefined, 12],
      ['ctx', 12, 13],
    ]);
    expect(diffStats(TWO_FILES)).toEqual({ added: 3, removed: 1 });
  });

  it('takes hunks without line numbers and absolute paths', () => {
    const files = parseDiff('--- /tmp/x.md\n+++ /tmp/x.md\n@@ @@\n-a\n+b\n');
    expect(files[0].path).toBe('/tmp/x.md');
    expect(files[0].hunks[0].lines).toEqual([{ kind: 'del', text: 'a' }, { kind: 'add', text: 'b' }]);
  });
});

describe('DiffViewer', () => {
  it('shows each file with its counts and shaded lines', () => {
    render(<DiffViewer diff={TWO_FILES} />);
    const a = screen.getByRole('region', { name: 'src/a.rs' });
    expect(within(a).getByText('+2')).toBeInTheDocument();
    expect(within(a).getByText('−1')).toBeInTheDocument();
    const added = within(a).getByText('more').closest('tr');
    expect(added).toHaveAttribute('data-kind', 'add');
    expect(added?.className).toContain('bg-diff-add');
    expect(within(screen.getByRole('region', { name: 'notes.txt' })).getByText('new')).toBeInTheDocument();
  });

  it('says when there is nothing', () => {
    render(<DiffViewer diff="" />);
    expect(screen.getByText('No changes.')).toBeInTheDocument();
  });
});
