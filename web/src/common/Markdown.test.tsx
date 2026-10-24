import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Markdown } from './Markdown';
import { TicketKeysContext } from './ticketKeys';

describe('Markdown', () => {
  it("links its projects' ticket keys to the ticket's path, not in code or links, and not other keys", () => {
    render(
      <TicketKeysContext.Provider value={['ALP', 'PDT']}>
        <Markdown>{'Fixes ALP-12 (see PDT-3, [the plan](https://x.test/PDT-4)); not `ALP-5`, UTF-8 or XALP-6.'}</Markdown>
      </TicketKeysContext.Provider>,
    );
    const links = screen.getAllByRole('link');
    expect(links.map((a) => [a.textContent, a.getAttribute('href'), a.getAttribute('target')])).toEqual([
      ['ALP-12', '/tickets/ALP-12', null],
      ['PDT-3', '/tickets/PDT-3', null],
      ['the plan', 'https://x.test/PDT-4', '_blank'],
    ]);
  });

  it('reads as a document with variant="document": roomier text, display-only task boxes', () => {
    const { container } = render(<Markdown variant="document">{'### Layout\n\n- [x] Ask Sam\n- [ ] Sketch it'}</Markdown>);
    expect(container.firstElementChild).toHaveAttribute('data-variant', 'document');
    expect(container.firstElementChild).toHaveClass('text-base');
    expect(screen.getByRole('heading', { name: 'Layout' })).toHaveClass('font-semibold');
    const boxes = screen.getAllByRole('checkbox');
    expect(boxes.map((b) => [b.getAttribute('aria-checked'), b.getAttribute('aria-disabled')])).toEqual([['true', 'true'], ['false', 'true']]);
  });

  it('links no ticket keys without the projects', () => {
    render(<Markdown>{'Fixes ALP-12'}</Markdown>);
    expect(screen.queryByRole('link')).toBeNull();
  });

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
