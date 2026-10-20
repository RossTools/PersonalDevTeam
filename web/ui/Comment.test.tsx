import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Comment, CommentComposer, CommentThread } from './Comment';

describe('Comment', () => {
  it('marks a draft and edits it in place', async () => {
    const onEdit = vi.fn();
    const onDelete = vi.fn();
    render(<Comment author="You" body="Rename this" draft onEdit={onEdit} onDelete={onDelete} />);
    expect(screen.getByRole('article', { name: 'Draft comment' })).toBeInTheDocument();
    expect(screen.getByText('Draft')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const box = screen.getByRole('textbox', { name: 'Edit comment' });
    await userEvent.clear(box);
    await userEvent.type(box, 'Rename it');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(onEdit).toHaveBeenCalledWith('Rename it');
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(onDelete).toHaveBeenCalled();
  });

  it('offers no editing once sent', () => {
    render(<Comment author="You" body="Sent" onEdit={vi.fn()} />);
    expect(screen.getByRole('article', { name: 'Comment' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
  });
});

describe('CommentComposer', () => {
  it('submits with Ctrl+Enter and cancels with Escape, never blank', async () => {
    const onSubmit = vi.fn();
    const onCancel = vi.fn();
    render(<CommentComposer label="Comment" action="Comment" onSubmit={onSubmit} onCancel={onCancel} />);
    expect(screen.getByRole('button', { name: 'Comment' })).toBeDisabled();
    const box = screen.getByRole('textbox', { name: 'Comment' });
    await userEvent.type(box, '  Looks off  ');
    await userEvent.keyboard('{Control>}{Enter}{/Control}');
    expect(onSubmit).toHaveBeenCalledWith('Looks off');
    expect(box).toHaveValue('');
    await userEvent.type(box, '{Escape}');
    expect(onCancel).toHaveBeenCalled();
  });
});

describe('CommentThread', () => {
  const comments = [
    { id: 'a', author: 'You', body: 'First', draft: false },
    { id: 'b', author: 'You', body: 'Second', draft: true },
  ];

  it('replies and resolves', async () => {
    const onReply = vi.fn();
    const onResolve = vi.fn();
    render(<CommentThread comments={comments} title="a.rs:3" onReply={onReply} onResolve={onResolve} onEdit={vi.fn()} />);
    expect(screen.getAllByRole('article')).toHaveLength(2);
    expect(screen.getAllByRole('button', { name: 'Edit' })).toHaveLength(1);
    await userEvent.click(screen.getByRole('button', { name: 'Reply' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Reply' }), 'Third');
    await userEvent.click(screen.getByRole('button', { name: 'Reply' }));
    expect(onReply).toHaveBeenCalledWith('Third');
    await userEvent.click(screen.getByRole('button', { name: 'Resolve' }));
    expect(onResolve).toHaveBeenCalledWith(true);
  });

  it('collapses when resolved and opens on a click', async () => {
    const onResolve = vi.fn();
    render(<CommentThread comments={comments} resolved onResolve={onResolve} />);
    expect(screen.queryByRole('article')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Resolved/ }));
    expect(screen.getAllByRole('article')).toHaveLength(2);
    await userEvent.click(screen.getByRole('button', { name: 'Reopen' }));
    expect(onResolve).toHaveBeenCalledWith(false);
  });
});
