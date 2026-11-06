import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { plugins } from '../api/gen/plugins';
import type { ExternalLink } from '../api/gen/ExternalLink';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { OpenTicketContext } from './actions';
import { ExternalKeys } from './ExternalKeys';
import { event, ticket } from './fixtures';
import { eventTitle } from './model';
import { TicketView } from './TicketView';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const manual: ExternalLink = {
  system: 'github', key: 'o/r#12', url: 'https://github.com/o/r/issues/12', origin: 'manual', closeOnDone: false,
  title: 'Crash on start', state: 'open',
};
const imported: ExternalLink = {
  system: 'jira', key: 'ABC-3', url: 'https://acme.atlassian.net/browse/ABC-3', origin: 'imported', closeOnDone: true,
  title: 'Old bug', state: 'closed',
};

function stub(t = ticket(1), extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    'GET /api/tickets/t1': t,
    'GET /api/tickets/t1/events': { items: [], nextCursor: null },
    'GET /api/integrations': plugins,
    'GET /api/tickets': { items: [], nextCursor: null, total: 0 },
    'POST /api/tickets/t1/issues/refresh': t,
    ...extra,
  });
}

const view = () =>
  renderWithQuery(
    <OpenTicketContext.Provider value={vi.fn()}>
      <TicketView ticketId="t1" />
    </OpenTicketContext.Provider>,
  );

describe('issue links', () => {
  it("shows each issue's tracker, title and state, and reads them again when the ticket opens", async () => {
    const calls = stub(ticket(1, { external: [manual, imported] }));
    view();
    const list = await screen.findByRole('list', { name: 'Issues' });
    expect(within(list).getByText('Crash on start')).toBeInTheDocument();
    expect(within(list).getByText('Open')).toBeInTheDocument();
    expect(within(list).getByText('Old bug')).toHaveClass('line-through');
    expect(within(list).getByText('Closed')).toBeInTheDocument();
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/tickets/t1/issues/refresh')).toBe(true));
  });

  it('reading the issues again marks no ticket list, timeline or labels stale: the ticket takes the answer', async () => {
    let t = ticket(1, { external: [manual] });
    stub(t, {
      'GET /api/tickets/t1': () => t,
      'POST /api/tickets/t1/issues/refresh': () => (t = ticket(1, { external: [{ ...manual, title: 'Crash on launch' }] })),
    });
    const { qc } = view();
    const invalidated = vi.spyOn(qc, 'invalidateQueries');
    const list = await screen.findByRole('list', { name: 'Issues' });
    expect(await within(list).findByText('Crash on launch')).toBeInTheDocument();
    expect(invalidated).not.toHaveBeenCalled();
  });

  it('offers closing on Done only for an issue linked by hand, off until ticked', async () => {
    const calls = stub(ticket(1, { external: [manual, imported] }), { 'PATCH /api/tickets/t1/issues': ticket(1) });
    view();
    const list = await screen.findByRole('list', { name: 'Issues' });
    const boxes = await within(list).findAllByRole('checkbox', { name: 'Close it when this ticket is done' });
    expect(boxes).toHaveLength(1);
    expect(boxes[0]).not.toBeChecked();
    await userEvent.click(boxes[0]);
    await waitFor(() =>
      expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ system: 'github', key: 'o/r#12', closeOnDone: true }),
    );
  });

  it('unlinks an issue from its menu', async () => {
    const calls = stub(ticket(1, { external: [manual] }), { 'DELETE /api/tickets/t1/issues': ticket(1) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Issue o/r#12' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Unlink' }));
    await waitFor(() =>
      expect(calls.find((c) => c.method === 'DELETE')?.path).toBe('/api/tickets/t1/issues?system=github&key=o%2Fr%2312'),
    );
  });

  it('links an issue by its key', async () => {
    const calls = stub(ticket(1), { 'POST /api/tickets/t1/issues': ticket(1, { external: [manual] }) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Link issue…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Link an issue to ALP-1' });
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Issue' }), ' o/r#12 {Enter}');
    await waitFor(() => expect(calls.find((c) => c.path === '/api/tickets/t1/issues' && c.method === 'POST')?.body).toEqual({ issue: 'o/r#12' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('adds an address no tracker knows as a plain link with a label', async () => {
    const withLink = ticket(1, { webLinks: [{ id: 'w1', url: 'https://www.figma.com/file/abc', label: 'Figma' }] });
    const calls = stub(ticket(1), {
      'POST /api/tickets/t1/issues': new Reply(422, { code: 'unrecognized', message: 'not an issue' }),
      'POST /api/tickets/t1/web-links': withLink,
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Link issue…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Link an issue to ALP-1' });
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Issue' }), 'https://www.figma.com/file/abc{Enter}');
    expect(await within(dialog).findByText(/No tracker recognizes this address/)).toBeInTheDocument();
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Label' }), 'Figma');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Add link' }));
    await waitFor(() =>
      expect(calls.find((c) => c.path === '/api/tickets/t1/web-links')?.body).toEqual({ url: 'https://www.figma.com/file/abc', label: 'Figma' }),
    );
  });

  it('shows plain links apart from issues, and removes one', async () => {
    const t = ticket(1, { webLinks: [{ id: 'w1', url: 'https://docs.example.com/x', label: 'Docs' }] });
    const calls = stub(t, { 'DELETE /api/tickets/t1/web-links/w1': ticket(1) });
    view();
    const list = await screen.findByRole('list', { name: 'Other links' });
    expect(within(list).getByRole('link', { name: 'Docs' })).toHaveAttribute('href', 'https://docs.example.com/x');
    expect(screen.queryByRole('list', { name: 'Issues' })).not.toBeInTheDocument();
    await userEvent.click(within(list).getByRole('button', { name: 'Link Docs' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Remove link' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tickets/t1/web-links/w1')).toBe(true));
  });

  it("names a chip's tracker, title and state", async () => {
    stubApi({ 'GET /api/integrations': plugins });
    renderWithQuery(<ExternalKeys links={[{ ...manual, state: 'closed' }]} linked />);
    const link = screen.getByRole('link', { name: /#12/ });
    await waitFor(() => expect(link.getAttribute('title')).toBe('GitHub issue o/r#12: Crash on start (closed)'));
    expect(link).toHaveClass('line-through');
  });

  it('says on the timeline what was linked and unlinked, naming the tracker from its plugin', async () => {
    const label = (system: string) => ({ github: 'GitHub', jira: 'Jira', gitlab: 'GitLab' })[system];
    const title = (kind: 'linked' | 'unlinked', data: Record<string, unknown>) => eventTitle(event('e', { kind, data }), label);
    expect(title('linked', { system: 'jira', key: 'ABC-3', url: 'u', origin: 'manual' })).toBe('Linked: Jira issue ABC-3');
    expect(title('unlinked', { system: 'github', key: 'o/r#12', url: 'u' })).toBe('Unlinked: GitHub issue o/r#12');
    expect(title('linked', { system: 'gitlab', key: 'g/app#4', url: 'u' })).toBe('Linked: GitLab issue g/app#4');
    expect(title('linked', { url: 'https://www.figma.com/file/abc', label: 'Figma' })).toBe('Linked: Figma');
    expect(title('unlinked', { url: 'https://docs.example.com/x', label: '' })).toBe('Unlinked: https://docs.example.com/x');
    expect(title('unlinked', { kind: 'blocks', direction: 'incoming', key: 'ALP-2' })).toBe('Unlinked: blocked by ALP-2');

    // In a ticket's timeline, the tracker's name comes from its plugin.
    stub(ticket(1), {
      'GET /api/tickets/t1/events': { items: [event('e1', { kind: 'unlinked', data: { system: 'jira', key: 'ABC-3', url: 'u' } })], nextCursor: null },
    });
    view();
    expect(await screen.findByText('Unlinked: Jira issue ABC-3')).toBeInTheDocument();
  });
});
