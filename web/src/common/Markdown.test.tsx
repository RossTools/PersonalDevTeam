import { render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { Markdown } from './Markdown';
import { MarkdownRefsContext, type MarkdownRefs } from './markdownRefs';
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

describe('Markdown in a conversation (refs)', () => {
  const forge = { kind: 'github', name: 'GitHub', repo: 'acme/shop', repoUrl: 'https://github.com/acme/shop', branchUrl: null, compareUrl: null };
  const refs: MarkdownRefs = { sessionId: 's1', projectKey: 'ALP', worktreeId: null, forge };
  const inConversation = (md: string) => renderWithQuery(
    <TicketKeysContext.Provider value={['ALP']}>
      <MarkdownRefsContext.Provider value={refs}>
        <Markdown>{md}</Markdown>
      </MarkdownRefsContext.Provider>
    </TicketKeysContext.Provider>,
  );
  afterEach(() => vi.unstubAllGlobals());

  it('shows ticket keys and PRs as pills, not inside code or links', () => {
    stubApi({});
    const { container } = inConversation('ALP-12 waits on PR #7 and https://github.com/acme/shop/pull/9; not `ALP-5`, [ALP-6](https://x.test) or a#3.\n\n```\nALP-8\n```');
    const pills = [...container.querySelectorAll('[data-slot=ticket-pill],[data-slot=pr-pill]')];
    expect(pills.map((p) => [p.textContent, p.getAttribute('href')])).toEqual([
      ['ALP-12', '/tickets/ALP-12'],
      ['PR #7', 'https://github.com/acme/shop/pull/7'],
      ['PR #9', 'https://github.com/acme/shop/pull/9'],
    ]);
    expect(screen.getByRole('link', { name: 'ALP-6' })).toHaveAttribute('href', 'https://x.test');
    expect(container.textContent).toContain('waits on PR #7 and');
    expect(screen.getByText('ALP-5').tagName).toBe('CODE');
  });

  it('shows commit SHAs in muted mono', () => {
    stubApi({});
    inConversation('Pushed 4427552e on top.');
    expect(screen.getByText('4427552e')).toHaveClass('text-muted-foreground', 'font-mono');
  });

  it('leaves SHAs and #numbers alone outside a conversation', () => {
    render(<Markdown>{'`4427552e` and #7'}</Markdown>);
    expect(screen.getByText('4427552e')).toHaveClass('bg-muted');
    expect(screen.queryByRole('link')).toBeNull();
  });

  it("links the session's files to the file viewer, and leaves other paths as code", async () => {
    stubApi({ 'GET /api/sessions/s1/files': (c: { path: string }) => (c.path.includes('real') ? ['src/real.ts'] : []) });
    inConversation('See `src/real.ts` and `src/gone.ts`.');
    const link = await screen.findByRole('link', { name: 'src/real.ts' });
    expect(link).toHaveAttribute('href', '/files/src/real.ts?project=ALP');
    expect(screen.getByText('src/gone.ts').closest('a')).toBeNull();
  });

  it('frames tables as a card with status chips', () => {
    stubApi({});
    const { container } = inConversation('| Ticket | State |\n|---|---|\n| ALP-1 | CI failed |\n| ALP-2 | Guest checkout |');
    expect(container.querySelector('[data-slot=table-card]')).not.toBeNull();
    expect(screen.getByText('CI failed')).toHaveAttribute('data-slot', 'badge');
    expect(screen.getByText('Guest checkout')).not.toHaveAttribute('data-slot', 'badge');
  });
});
