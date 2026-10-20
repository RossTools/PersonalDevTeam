import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Check, ChevronRight, RotateCcw } from 'lucide-react';
import { Badge } from './Badge';
import { Button } from './Button';
import { Textarea } from './Textarea';
import { cn } from './lib/utils';

export interface CommentComposerProps {
  /** The text area's accessible name. */
  label: string;
  initial?: string;
  /** The submit button's label ("Comment", "Reply", "Save"). */
  action: string;
  /** Resolves when saved; the composer clears (and stays for another) unless `onCancel` closes it. */
  onSubmit: (text: string) => Promise<unknown> | void;
  onCancel?: () => void;
  autoFocus?: boolean;
  placeholder?: string;
  className?: string;
}

/** A text area with submit (also Ctrl/⌘+Enter) and Cancel (also Escape). Blank text can't be submitted. */
export function CommentComposer({ label, initial = '', action, onSubmit, onCancel, autoFocus, placeholder, className }: CommentComposerProps) {
  const [text, setText] = useState(initial);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (autoFocus) ref.current?.focus();
  }, [autoFocus]);
  const submit = async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    try {
      await onSubmit(text.trim());
      setText('');
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <Textarea
        ref={ref}
        aria-label={label}
        value={text}
        placeholder={placeholder}
        rows={2}
        className="min-h-14 text-xs"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void submit();
          } else if (e.key === 'Escape' && onCancel) {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
      />
      <div className="flex justify-end gap-1.5">
        {onCancel && <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>}
        <Button size="sm" variant="primary" disabled={!text.trim()} pending={busy} onClick={() => void submit()}>
          {action}
        </Button>
      </div>
    </div>
  );
}

export interface CommentProps {
  author: ReactNode;
  /** "3 min ago". */
  time?: ReactNode;
  body: string;
  /** Not sent yet: dashed outline and a "Draft" badge; Edit and Delete are offered when their handlers are given. */
  draft?: boolean;
  onEdit?: (text: string) => Promise<unknown> | void;
  onDelete?: () => void;
  /** Renders the body (Markdown); plain text by default. */
  renderBody?: (text: string) => ReactNode;
  className?: string;
}

/** One comment: author, time, body; a draft can be edited in place or deleted. */
export function Comment({ author, time, body, draft = false, onEdit, onDelete, renderBody, className }: CommentProps) {
  const [editing, setEditing] = useState(false);
  return (
    <article
      aria-label={draft ? 'Draft comment' : 'Comment'}
      className={cn('flex flex-col gap-1 rounded-md border px-2.5 py-2', draft ? 'border-dashed border-primary' : 'border-border bg-card', className)}
    >
      <header className="flex items-center gap-1.5 text-meta text-muted-foreground">
        <span className="font-medium text-foreground">{author}</span>
        {time && <span>{time}</span>}
        {draft && <Badge tone="accent">Draft</Badge>}
        {draft && !editing && (onEdit || onDelete) && (
          <span className="ml-auto flex gap-0.5">
            {onEdit && <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>Edit</Button>}
            {onDelete && <Button size="sm" variant="ghost" onClick={onDelete}>Delete</Button>}
          </span>
        )}
      </header>
      {editing && onEdit ? (
        <CommentComposer
          label="Edit comment"
          initial={body}
          action="Save"
          autoFocus
          onSubmit={async (t) => {
            await onEdit(t);
            setEditing(false);
          }}
          onCancel={() => setEditing(false)}
        />
      ) : (
        <div className="text-xs">{renderBody ? renderBody(body) : <p className="whitespace-pre-wrap">{body}</p>}</div>
      )}
    </article>
  );
}

export interface ThreadComment {
  id: string;
  author: ReactNode;
  time?: ReactNode;
  body: string;
  draft: boolean;
}

export interface CommentThreadProps {
  /** The first comment, then its replies. */
  comments: ThreadComment[];
  /** Where the thread is ("src/a.rs:12"), shown above it when given. */
  title?: ReactNode;
  resolved?: boolean;
  /** Resolve / Reopen (the first comment's thread). */
  onResolve?: (resolved: boolean) => void;
  onReply?: (text: string) => Promise<unknown> | void;
  onEdit?: (id: string, text: string) => Promise<unknown> | void;
  onDelete?: (id: string) => void;
  renderBody?: (text: string) => ReactNode;
  /** A note under the title (e.g. "The lines changed since"). */
  note?: ReactNode;
  className?: string;
}

/** A comment thread: its comments, Reply, and Resolve; a resolved thread collapses to one line until opened. */
export function CommentThread({ comments, title, resolved = false, onResolve, onReply, onEdit, onDelete, renderBody, note, className }: CommentThreadProps) {
  const [open, setOpen] = useState(!resolved);
  const [replying, setReplying] = useState(false);
  const wasResolved = useRef(resolved);
  useEffect(() => {
    // Collapse when it becomes resolved, open when reopened; leave the user's own toggling alone otherwise.
    if (resolved !== wasResolved.current) setOpen(!resolved);
    wasResolved.current = resolved;
  }, [resolved]);
  const first = comments[0];
  if (!first) return null;
  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className={cn('flex w-full items-center gap-1.5 rounded-md border border-border bg-card px-2 py-1 text-left text-meta text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring', className)}
      >
        <ChevronRight aria-hidden className="size-3" />
        <Check aria-hidden className="size-3 text-running" />
        <span className="shrink-0">Resolved</span>
        {title && <span className="shrink-0 font-mono">{title}</span>}
        <span className="min-w-0 truncate">{first.body}</span>
      </button>
    );
  }
  return (
    <section aria-label={title ? `Thread on ${typeof title === 'string' ? title : 'a line'}` : 'Thread'} className={cn('flex flex-col gap-1.5', className)}>
      {(title || note) && (
        <div className="flex flex-col text-meta text-muted-foreground">
          {title && <span className="font-mono">{title}</span>}
          {note}
        </div>
      )}
      {comments.map((c) => (
        <Comment
          key={c.id}
          author={c.author}
          time={c.time}
          body={c.body}
          draft={c.draft}
          renderBody={renderBody}
          {...(c.draft && onEdit ? { onEdit: (t: string) => onEdit(c.id, t) } : {})}
          {...(c.draft && onDelete ? { onDelete: () => onDelete(c.id) } : {})}
        />
      ))}
      {replying && onReply ? (
        <CommentComposer
          label="Reply"
          action="Reply"
          autoFocus
          onSubmit={async (t) => {
            await onReply(t);
            setReplying(false);
          }}
          onCancel={() => setReplying(false)}
        />
      ) : (
        (onReply || onResolve) && (
          <div className="flex gap-1">
            {onReply && <Button size="sm" variant="ghost" onClick={() => setReplying(true)}>Reply</Button>}
            {onResolve && (
              resolved ? (
                <Button size="sm" variant="ghost" onClick={() => onResolve(false)}><RotateCcw aria-hidden /> Reopen</Button>
              ) : (
                <Button size="sm" variant="ghost" onClick={() => onResolve(true)}><Check aria-hidden /> Resolve</Button>
              )
            )}
          </div>
        )
      )}
    </section>
  );
}
