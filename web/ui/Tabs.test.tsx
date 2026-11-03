import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { TabPanel, Tabs } from './Tabs';

const tabs = [
  { id: 'overview', label: 'Overview' },
  { id: 'conversation', label: 'Conversation' },
  { id: 'changes', label: 'Changes', disabled: true },
] as const;

function Demo({ onChange }: { onChange?: (id: string) => void }) {
  const [tab, setTab] = useState<string>('overview');
  return (
    <Tabs aria-label="Ticket sections" tabs={[...tabs]} value={tab} onChange={(t) => { setTab(t); onChange?.(t); }}>
      <TabPanel id="overview">The description</TabPanel>
      <TabPanel id="conversation">The sessions</TabPanel>
      <TabPanel id="changes">The diff</TabPanel>
    </Tabs>
  );
}

describe('Tabs', () => {
  it('shows the selected tab and its panel, labelled by the tab', () => {
    render(<Demo />);
    expect(screen.getByRole('tablist', { name: 'Ticket sections' })).toBeInTheDocument();
    const tab = screen.getByRole('tab', { name: 'Overview' });
    expect(tab).toHaveAttribute('aria-selected', 'true');
    expect(tab.className).toContain('data-selected:border-primary');
    const panel = screen.getByRole('tabpanel');
    expect(panel).toHaveTextContent('The description');
    expect(panel).toHaveAttribute('aria-labelledby', tab.id);
    expect(screen.queryByText('The sessions')).toBeNull();
  });

  it('a click shows another tab', async () => {
    const onChange = vi.fn();
    render(<Demo onChange={onChange} />);
    await userEvent.click(screen.getByRole('tab', { name: 'Conversation' }));
    expect(onChange).toHaveBeenCalledWith('conversation');
    expect(screen.getByRole('tabpanel')).toHaveTextContent('The sessions');
  });

  it('arrow keys move and show, skipping a disabled tab; the list is one Tab stop', async () => {
    render(<Demo />);
    await userEvent.tab();
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveFocus();
    await userEvent.keyboard('{ArrowRight}');
    expect(screen.getByRole('tab', { name: 'Conversation' })).toHaveFocus();
    expect(screen.getByRole('tabpanel')).toHaveTextContent('The sessions');
    await userEvent.keyboard('{ArrowRight}');
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveFocus();
    expect(screen.getByRole('tab', { name: 'Changes' })).toHaveAttribute('aria-disabled', 'true');
    await userEvent.tab();
    expect(screen.getByRole('tabpanel')).toHaveFocus();
  });
});
