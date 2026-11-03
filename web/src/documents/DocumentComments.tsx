import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { MessageSquarePlus } from 'lucide-react';
import { Button, CommentComposer, CommentThread, RelativeTime, SectionLabel, Select, Skeleton } from '@ui';
import { api } from '../api/client';
import type { CommentAnchor } from '../api/gen/CommentAnchor';
import { Markdown } from '../common/Markdown';
import { documentKeys, useDocumentComments } from './queries';
import { LoadError } from '../common/QueryStatus';

/** A doc's headings (`#` lines outside code fences), for comments on them. */
export function headingsOf(markdown: string): string[] {
  const out: string[] = [];
  let fenced = false;
  for (const line of markdown.split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const m = !fenced && /^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$/.exec(line);
    if (m && !out.includes(m[1]!)) out.push(m[1]!);
  }
  return out;
}

/** An anchor for people: `Goal`, `“some text”`, `Sheet1!B3`. */
export function anchorLabel(a: CommentAnchor): string {
  switch (a.on) {
    case 'heading': return a.heading;
    case 'text': return `“${a.quote}”`;
    case 'cell': return `${a.sheet}!${a.range}`;
  }
}

export interface DocumentCommentsProps {
  id: string;
  /** A doc's headings to comment on. */
  headings?: string[];
  /** A sheet's selection now (where a new comment goes), read when the user starts one. */
  selection?: () => CommentAnchor | null;
  /** Shows where a comment is (a sheet selects its cells). */
  onShow?: (a: CommentAnchor) => void;
}

const WHOLE = '__whole__';
const QUOTE = '__quote__';

/**
 * A document's comments (PDT-315), kept by PDT: each on a heading or some text of a doc, or a cell or range of a
 * sheet; reply and resolve. A new comment on a doc goes on the heading picked, or on the text selected on the page; on
 * a sheet, on the cells selected.
 */
export function DocumentComments({ id, headings, selection, onShow }: DocumentCommentsProps) {
  const qc = useQueryClient();
  const comments = useDocumentComments(id, true);
  const [adding, setAdding] = useState<{ anchor: CommentAnchor | null; quote: string | null } | null>(null);
  const [pick, setPick] = useState<string>(WHOLE);
  const settle = () => qc.invalidateQueries({ queryKey: documentKeys.comments(id) });

  const start = () => {
    if (selection) {
      setAdding({ anchor: selection(), quote: null });
      return;
    }
    const quote = window.getSelection()?.toString().trim().slice(0, 500) || null;
    setPick(quote ? QUOTE : WHOLE);
    setAdding({ anchor: null, quote });
  };
  const anchorOf = (): CommentAnchor | null => {
    if (!adding) return null;
    if (selection) return adding.anchor;
    if (pick === QUOTE && adding.quote) return { on: 'text', quote: adding.quote };
    if (pick !== WHOLE && pick !== QUOTE) return { on: 'heading', heading: pick };
    return null;
  };

  const open = (comments.data ?? []).filter((c) => !c.resolved).length;
  return (
    <section aria-label="Comments" className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <SectionLabel as="h2">Comments{open > 0 && ` · ${open}`}</SectionLabel>
        {!adding && <Button variant="ghost" size="sm" compact className="ml-auto" onClick={start}><MessageSquarePlus aria-hidden /> Comment</Button>}
      </div>
      {adding && (
        <div className="flex flex-col gap-1.5 rounded-md border border-border bg-card p-2">
          {selection ? (
            <p className="text-meta text-muted-foreground">On {adding.anchor ? anchorLabel(adding.anchor) : 'the sheet'}</p>
          ) : (
            <Select
              aria-label="Comment on"
              size="sm"
              options={[
                { value: WHOLE, label: 'The whole document' },
                ...(adding.quote ? [{ value: QUOTE, label: `The selected text: “${adding.quote.slice(0, 40)}${adding.quote.length > 40 ? '…' : ''}”` }] : []),
                ...(headings ?? []).map((h) => ({ value: h, label: `Heading: ${h}` })),
              ]}
              value={pick}
              onChange={(v) => setPick(v)}
            />
          )}
          <CommentComposer
            label="New comment"
            action="Comment"
            autoFocus
            onCancel={() => setAdding(null)}
            onSubmit={async (body) => {
              const anchor = anchorOf();
              await api.addDocumentComment(id, { body, ...(anchor ? { anchor } : {}) });
              setAdding(null);
              await settle();
            }}
          />
        </div>
      )}
      {comments.isPending && <Skeleton lines={2} label="Loading comments" />}
      <LoadError what="The comments" error={comments.error} />
      {comments.data?.length === 0 && !adding && <p className="text-xs text-muted-foreground">No comments yet.</p>}
      {comments.data?.map((c) => (
        <CommentThread
          key={c.id}
          title={c.anchor ? (
            onShow ? <button type="button" className="hover:underline" onClick={() => onShow(c.anchor!)}>{anchorLabel(c.anchor)}</button> : anchorLabel(c.anchor)
          ) : c.quote ?? undefined}
          resolved={c.resolved}
          comments={[
            { id: c.id, author: c.author, time: c.createdAt ? <RelativeTime time={c.createdAt} /> : undefined, body: c.body, draft: false },
            ...c.replies.map((r) => ({ id: r.id, author: r.author, time: r.createdAt ? <RelativeTime time={r.createdAt} /> : undefined, body: r.body, draft: false })),
          ]}
          onReply={async (body) => {
            await api.replyToDocumentComment(id, c.id, body);
            await settle();
          }}
          onResolve={(resolved) => void (async () => {
            await api.resolveDocumentComment(id, c.id, resolved);
            await settle();
          })()}
          renderBody={(t) => <Markdown>{t}</Markdown>}
        />
      ))}
    </section>
  );
}
