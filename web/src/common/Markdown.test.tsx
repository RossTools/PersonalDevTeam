import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Markdown } from './Markdown';

describe('Markdown', () => {
  it('renders a GitHub table', () => {
    render(<Markdown>{'| Stage | Count |\n|---|---|\n| Draft | 3 |\n| Ready | 5 |'}</Markdown>);
    const table = screen.getByRole('table');
    expect(within(table).getByRole('columnheader', { name: 'Stage' })).toBeInTheDocument();
    expect(within(table).getByRole('cell', { name: '5' })).toBeInTheDocument();
  });

  it('renders a task list with read-only checkboxes', () => {
    render(<Markdown>{'- [x] schema\n- [ ] API'}</Markdown>);
    const boxes = screen.getAllByRole('checkbox');
    expect(boxes).toHaveLength(2);
    expect(boxes[0]).toBeChecked();
    expect(boxes[1]).not.toBeChecked();
    expect(boxes[0]).toBeDisabled();
  });

  it('keeps single line breaks only with breaks', () => {
    const src = '{\n  "type": "system"\n}';
    const { container, rerender } = render(<Markdown>{src}</Markdown>);
    expect(container.querySelectorAll('br')).toHaveLength(0);
    rerender(<Markdown breaks>{src}</Markdown>);
    expect(container.querySelectorAll('br')).toHaveLength(2);
  });

  it('renders strikethrough and inline code', () => {
    const { container } = render(<Markdown>{'~~old~~ and `pdtd`'}</Markdown>);
    expect(container.querySelector('del')).toHaveTextContent('old');
    expect(container.querySelector('code')?.className).toContain('font-mono');
  });

  it('opens links in a new tab with rel=noreferrer', () => {
    render(<Markdown>{'See [the spec](https://example.com/spec) and https://example.com/auto'}</Markdown>);
    const link = screen.getByRole('link', { name: 'the spec' });
    expect(link).toHaveAttribute('href', 'https://example.com/spec');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
    expect(screen.getByRole('link', { name: 'https://example.com/auto' })).toHaveAttribute('target', '_blank');
  });

  it('never renders raw HTML or javascript: links', () => {
    const { container } = render(
      <Markdown>{'<script>alert(1)</script>\n\n<img src=x onerror="alert(1)">\n\n[x](javascript:alert(1))'}</Markdown>,
    );
    expect(container.querySelector('script')).toBeNull();
    expect(container).toHaveTextContent('<script>alert(1)</script>');
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('[onerror]')).toBeNull();
    const link = container.querySelector('a');
    expect(link?.getAttribute('href') ?? '').not.toContain('javascript:');
  });

  it('sizes headings for a ticket body', () => {
    render(<Markdown>{'# Title'}</Markdown>);
    expect(screen.getByRole('heading', { level: 1, name: 'Title' }).className).toContain('text-base');
  });
});
