import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError } from '../api/client';
import type { TicketAttachment } from '../api/gen/TicketAttachment';
import type { TicketEvent } from '../api/gen/TicketEvent';
import { Markdown } from '../common/Markdown';
import { renderWithQuery, stubApi } from '../test-utils';
import { event } from './fixtures';
import { TicketAttachments } from './attachments';
import { TicketTimeline } from './TicketParts';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const shot: TicketAttachment = {
  id: 'a1', ticketId: 't1', name: 'shot.png', mediaType: 'image/png', size: 2048, actor: 'user', agent: null,
  eventId: null, createdAt: 1,
};

function paste(box: HTMLElement, file: File) {
  fireEvent.paste(box, { clipboardData: { files: [file], types: ['Files'], getData: () => '' } });
}

describe('ticket attachments', () => {
  it('a screenshot pasted into a comment is uploaded and shows inline once sent', async () => {
    const events: TicketEvent[] = [];
    stubApi({
      'GET /api/tickets/t1/events': () => ({ items: [...events].reverse(), nextCursor: null }),
      'GET /api/tickets/t1/attachments': [],
      'POST /api/tickets/t1/comments': (c) => {
        const e = event('e1', { kind: 'comment', actor: 'user', body: (c.body as { body: string }).body });
        events.push(e);
        return e;
      },
    });
    const upload = vi.spyOn(api, 'uploadTicketAttachment').mockImplementation(async (_id, _f, _name, onProgress) => {
      onProgress?.(0.5);
      return shot;
    });
    renderWithQuery(<TicketTimeline ticketId="t1" composer />);
    const box = await screen.findByRole('textbox', { name: 'Comment' });
    await userEvent.type(box, 'It breaks here:');
    paste(box, new File([new Uint8Array([0x89, 0x50])], 'image.png', { type: 'image/png' }));
    await waitFor(() => expect(box).toHaveValue('It breaks here:\n![shot.png](/api/tickets/t1/attachments/a1)'));
    expect(upload).toHaveBeenCalledOnce();
    expect(upload.mock.calls[0]![2]).toMatch(/^screenshot-.*\.png$/);
    await userEvent.click(screen.getByRole('button', { name: 'Comment' }));
    const img = await screen.findByRole('img', { name: 'shot.png' });
    expect(img).toHaveAttribute('src', '/api/tickets/t1/attachments/a1');
    await userEvent.click(screen.getByRole('button', { name: 'Open shot.png full size' }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
  });

  it('a failed upload says why and stays until dismissed', async () => {
    stubApi({ 'GET /api/tickets/t1/events': { items: [], nextCursor: null }, 'GET /api/tickets/t1/attachments': [] });
    vi.spyOn(api, 'uploadTicketAttachment').mockRejectedValue(
      new ApiError(400, 'unsupported_attachment', 'only images and UTF-8 text files can be attached'),
    );
    renderWithQuery(<TicketTimeline ticketId="t1" composer />);
    const box = await screen.findByRole('textbox', { name: 'Comment' });
    paste(box, new File(['\0'], 'a.bin', { type: 'application/octet-stream' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('only images and UTF-8 text files');
    expect(box).toHaveValue('');
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss a.bin' }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('lists the attachments and removes one after asking', async () => {
    let list: TicketAttachment[] = [shot, { ...shot, id: 'a2', name: 'build.log', mediaType: 'text/plain', size: 12 }];
    const calls = stubApi({
      'GET /api/tickets/t1/attachments': () => list,
      'DELETE /api/tickets/t1/attachments/a2': () => { list = [shot]; },
    });
    renderWithQuery(<TicketAttachments ticketId="t1" />);
    expect(await screen.findByRole('img', { name: 'shot.png' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'build.log' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Remove build.log' }));
    const confirm = await screen.findByRole('alertdialog');
    await userEvent.click(within(confirm).getByRole('button', { name: 'Remove attachment' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'build.log' })).not.toBeInTheDocument());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(true);
  });

  it('a text attachment opens in a viewer as plain text', async () => {
    stubApi({ 'GET /api/tickets/t1/attachments/a2': new Response('<b>not bold</b>', { status: 200 }) });
    renderWithQuery(<Markdown>{'See [build.log](/api/tickets/t1/attachments/a2)'}</Markdown>);
    await userEvent.click(screen.getByRole('button', { name: 'build.log' }));
    const dialog = await screen.findByRole('dialog');
    expect(await within(dialog).findByText('<b>not bold</b>')).toBeInTheDocument();
  });
});
