import { useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { FileText } from 'lucide-react';
import { Alert, Button, Dialog } from '@ui';

/** A ticket attachment's URL (`/api/tickets/<ticket>/attachments/<id>`), as Markdown links to it: its ids, or null. */
export function attachmentRef(href: string | undefined): { ticketId: string; id: string } | null {
  const m = href?.match(/^\/api\/tickets\/([^/?#]+)\/attachments\/([^/?#]+)$/);
  return m ? { ticketId: decodeURIComponent(m[1]!), id: decodeURIComponent(m[2]!) } : null;
}

/** An attached image shown inline (a ticket's description or comment); a click opens it full size. */
export function AttachmentImage({ src, alt }: { src: string; alt: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={`Open ${alt || 'the image'} full size`}
        className="my-2 block max-w-full cursor-zoom-in rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <img src={src} alt={alt} className="max-h-96 max-w-full rounded-md border border-border" />
      </button>
      {open && (
        <Dialog
          open
          onOpenChange={setOpen}
          title={alt || 'Image'}
          size="full"
          headerActions={<a href={src} target="_blank" rel="noreferrer" className="ml-auto text-xs text-primary hover:underline">Open in a new tab</a>}
        >
          <img src={src} alt={alt} className="mx-auto max-w-full" />
        </Dialog>
      )}
    </>
  );
}

/** A text file's content, fetched as plain text. */
function TextViewer({ src }: { src: string }) {
  const q = useQuery({
    queryKey: ['attachmentText', src],
    queryFn: async () => {
      const res = await fetch(src);
      if (!res.ok) throw new Error(res.status === 404 ? 'This file is no longer attached.' : `HTTP ${res.status}`);
      return res.text();
    },
    staleTime: Infinity,
  });
  if (q.isPending) return <p className="text-xs text-muted-foreground">Loading…</p>;
  if (q.isError) return <Alert tone="error">{q.error.message}</Alert>;
  return (
    <pre className="overflow-auto rounded-md border border-border bg-surface-2 p-2 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground">
      {q.data}
    </pre>
  );
}

/** An attached text file: a chip that opens it in a viewer. */
export function AttachmentLink({ src, name, children }: { src: string; name: string; children?: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="inline-flex h-5 max-w-full min-w-0 items-center gap-1 rounded-md bg-muted px-1.5 align-middle font-mono text-meta text-foreground outline-none hover:bg-accent hover:text-primary focus-visible:ring-2 focus-visible:ring-ring"
      >
        <FileText aria-hidden className="size-3 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate">{children ?? name}</span>
      </button>
      {open && (
        <Dialog
          open
          onOpenChange={setOpen}
          title={name}
          size="xl"
          footer={
            <>
              <a href={src} target="_blank" rel="noreferrer" className="mr-auto self-center text-xs text-primary hover:underline">Open raw</a>
              <Button onClick={() => setOpen(false)}>Close</Button>
            </>
          }
        >
          <TextViewer src={src} />
        </Dialog>
      )}
    </>
  );
}
