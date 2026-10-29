import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import { renderWithQuery, stubApi } from '../test-utils';
import { RIGHT_PANEL_OPEN_KEY, RIGHT_PANEL_WIDTH, RIGHT_PANEL_WIDTH_KEY, RightPanel, useRightPanelKey, useRightPanelOpen } from './RightPanel';

const projects = [{ id: 'p1', key: 'PX', name: 'P' }] as Project[];
const todo = (id: string, done = false, projectId: string | null = null): Todo => ({
  id, title: `Todo ${id}`, position: 0, done, projectId, ticketId: null, createdAt: 1, updatedAt: 1,
  doneAt: done ? 2 : null, projectKey: null, ticketKey: null, ticketTitle: null, body: '', dueOn: null, dueTime: null, recurrence: null,
});

/** The panel as AppShell holds it: its open state from storage, ⌘. toggling it. */
function Harness({ projectId = null }: { projectId?: string | null }) {
  const [open, setOpen] = useRightPanelOpen();
  useRightPanelKey(() => setOpen((o) => !o));
  return <RightPanel open={open} onOpenChange={setOpen} projectId={projectId} projects={projects} />;
}

beforeEach(() => {
  localStorage.clear();
  stubApi({
    'GET /api/todos': [todo('a'), todo('b', false, 'p1'), todo('c', true)],
    'GET /api/projects': [],
  });
});
afterEach(() => vi.unstubAllGlobals());

const panel = () => screen.getByRole('complementary', { name: 'Right panel' });

describe('RightPanel', () => {
  it('collapses to a strip with the open count, and opens from it, its button and ⌘.', async () => {
    renderWithQuery(<Harness />);
    const strip = await screen.findByRole('button', { name: 'Open the todo list (2 open)' });
    expect(within(panel()).queryByRole('heading', { name: 'Todos' })).toBeNull();
    await userEvent.click(strip);
    expect(within(panel()).getByRole('heading', { name: 'Todos' })).toBeInTheDocument();
    expect(await within(panel()).findByText('Todo a')).toBeInTheDocument();
    expect(localStorage.getItem(RIGHT_PANEL_OPEN_KEY)).toBe('true');
    await userEvent.click(screen.getByRole('button', { name: 'Collapse the right panel' }));
    expect(screen.getByRole('button', { name: /Open the todo list/ })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: '.', metaKey: true });
    expect(within(panel()).getByRole('heading', { name: 'Todos' })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: '.', metaKey: true });
    expect(within(panel()).queryByRole('heading', { name: 'Todos' })).toBeNull();
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

  it('is Todos only: no tabs, the open count, and a scope dropdown of All projects or the one on screen', async () => {
    localStorage.setItem(RIGHT_PANEL_OPEN_KEY, 'true');
    renderWithQuery(<Harness projectId="p1" />);
    expect(await within(panel()).findByText('Todo a')).toBeInTheDocument();
    expect(within(panel()).queryByRole('tab')).toBeNull();
    expect(within(panel()).queryByText('Notes')).toBeNull();
    expect(within(panel()).getByLabelText('2 open')).toHaveTextContent('2');
    await userEvent.click(within(panel()).getByRole('button', { name: 'Scope: All projects' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'PX' }));
    expect(within(panel()).queryByText('Todo a')).toBeNull();
    expect(within(panel()).getByText('Todo b')).toBeInTheDocument();
    expect(localStorage.getItem('pdt.todos.filter')).toBe('project');
    await userEvent.click(within(panel()).getByRole('button', { name: 'Scope: PX' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'All projects' }));
    expect(within(panel()).getByText('Todo a')).toBeInTheDocument();
  });

  it('focuses the add box when asked (Ctrl+] t)', async () => {
    localStorage.setItem(RIGHT_PANEL_OPEN_KEY, 'true');
    const { rerender } = renderWithQuery(<RightPanel open onOpenChange={() => {}} projectId={null} projects={projects} focusAddSeq={0} />);
    const box = await screen.findByRole('textbox', { name: 'Add a todo' });
    expect(box).not.toHaveFocus();
    rerender(<RightPanel open onOpenChange={() => {}} projectId={null} projects={projects} focusAddSeq={1} />);
    await waitFor(() => expect(box).toHaveFocus());
  });
});
