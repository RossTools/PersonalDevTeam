import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Todo } from '../api/gen/Todo';
import { renderWithQuery, stubApi } from '../test-utils';
import { RIGHT_PANEL_OPEN_KEY, RIGHT_PANEL_WIDTH, RIGHT_PANEL_WIDTH_KEY, RightPanel, useRightPanelKey, useRightPanelOpen } from './RightPanel';

const todo = (id: string, done = false): Todo => ({
  id, title: `Todo ${id}`, position: 0, done, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
  doneAt: done ? 2 : null, projectKey: null, ticketKey: null, ticketTitle: null,
});

/** The panel as AppShell holds it: its open state from storage, ⌘. toggling it. */
function Harness({ onOpenNote = () => {} }: { onOpenNote?: (id: string) => void }) {
  const [open, setOpen] = useRightPanelOpen();
  useRightPanelKey(() => setOpen((o) => !o));
  return <RightPanel open={open} onOpenChange={setOpen} projectId={null} projects={[]} onOpenNote={onOpenNote} />;
}

beforeEach(() => {
  localStorage.clear();
  stubApi({
    'GET /api/todos': [todo('a'), todo('b'), todo('c', true)],
    'GET /api/pm/notes': [{ id: 'n1', projectId: null, kind: 'user', title: 'Dark mode', body: '', pinned: false, sessionId: null, createdAt: 1, updatedAt: 1 }],
    'GET /api/projects': [],
  });
});
afterEach(() => vi.unstubAllGlobals());

const panel = () => screen.getByRole('complementary', { name: 'Right panel' });

describe('RightPanel', () => {
  it('collapses to a strip with the open count, and opens from it, its button and ⌘.', async () => {
    renderWithQuery(<Harness />);
    const strip = await screen.findByRole('button', { name: 'Open the todo list (2 open)' });
    expect(within(panel()).queryByRole('tab')).toBeNull();
    await userEvent.click(strip);
    expect(within(panel()).getByRole('tab', { name: 'Todos' })).toHaveAttribute('aria-selected', 'true');
    expect(await within(panel()).findByText('Todo a')).toBeInTheDocument();
    expect(localStorage.getItem(RIGHT_PANEL_OPEN_KEY)).toBe('true');
    await userEvent.click(screen.getByRole('button', { name: 'Collapse the right panel' }));
    expect(screen.getByRole('button', { name: /Open the todo list/ })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: '.', metaKey: true });
    expect(within(panel()).getByRole('tab', { name: 'Todos' })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: '.', metaKey: true });
    expect(within(panel()).queryByRole('tab')).toBeNull();
    expect(localStorage.getItem(RIGHT_PANEL_OPEN_KEY)).toBe('false');
  });

  it('keeps its open state and width across a reload', async () => {
    localStorage.setItem(RIGHT_PANEL_OPEN_KEY, 'true');
    const { unmount } = renderWithQuery(<Harness />);
    const divider = screen.getByRole('separator', { name: 'Resize the right panel' });
    expect(divider).toHaveAttribute('aria-valuenow', String(RIGHT_PANEL_WIDTH.default));
    divider.focus();
    await userEvent.keyboard('{ArrowLeft}{ArrowLeft}');
    expect(divider).toHaveAttribute('aria-valuenow', String(RIGHT_PANEL_WIDTH.default + 32));
    expect(localStorage.getItem(RIGHT_PANEL_WIDTH_KEY)).toBe(String(RIGHT_PANEL_WIDTH.default + 32));
    unmount();
    renderWithQuery(<Harness />);
    expect(panel()).toHaveStyle({ width: `${RIGHT_PANEL_WIDTH.default + 32}px` });
  });

  it('starts closed and at the default width when storage is blocked', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
    renderWithQuery(<Harness />);
    await userEvent.click(await screen.findByRole('button', { name: /Open the todo list/ }));
    await waitFor(() => expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', String(RIGHT_PANEL_WIDTH.default)));
    vi.restoreAllMocks();
  });

  it('has a Notes tab beside Todos that opens one of my notes', async () => {
    localStorage.setItem(RIGHT_PANEL_OPEN_KEY, 'true');
    const onOpenNote = vi.fn();
    renderWithQuery(<Harness onOpenNote={onOpenNote} />);
    const tabs = within(panel()).getAllByRole('tab').map((t) => t.textContent);
    expect(tabs).toEqual(['Todos', 'Notes']);
    await userEvent.click(within(panel()).getByRole('tab', { name: 'Notes' }));
    expect(within(panel()).getByRole('textbox', { name: 'Note title' })).toBeInTheDocument();
    await userEvent.click(await within(panel()).findByRole('button', { name: /Dark mode/ }));
    expect(onOpenNote).toHaveBeenCalledWith('n1');
  });
});
