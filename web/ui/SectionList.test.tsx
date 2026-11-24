import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Badge } from './Badge';
import { Button } from './Button';
import { SectionItem, SectionList } from './SectionList';

describe('SectionList', () => {
  it('lists items with their name, badges, actions, notice and meta line', () => {
    render(
      <SectionList aria-label="Instances">
        <SectionItem title="win-box" badges={<Badge>online</Badge>} actions={<Button size="sm">Remove…</Button>} notice={<p>Its key expired.</p>} meta={<><span>Windows</span><span>8 CPUs</span></>} />
        <SectionItem title={<code>main</code>} />
      </SectionList>,
    );
    const items = within(screen.getByRole('list', { name: 'Instances' })).getAllByRole('listitem');
    expect(items).toHaveLength(2);
    expect(within(items[0]).getByText('win-box')).toHaveClass('font-medium');
    expect(within(items[0]).getByText('online')).toBeInTheDocument();
    expect(within(items[0]).getByRole('button', { name: 'Remove…' })).toBeInTheDocument();
    expect(items[0]).toHaveTextContent('Its key expired.Windows8 CPUs');
    expect(within(items[1]).getByText('main').tagName).toBe('CODE');
  });

  it('pads its own items inside a Section', () => {
    render(<SectionList><SectionItem title="a" /></SectionList>);
    expect(screen.getByRole('list')).toHaveAttribute('data-flush');
  });
});
