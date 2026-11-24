import { useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { FileText } from 'lucide-react';
import { Alert, Button, Dialog, Link, Skeleton } from '@ui';
import { downloadUrl } from './download';
import { ticketKeys } from '../tickets/queries';

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
          onClose={() => setOpen(false)}
          title={alt || 'Image'}
          size="full"
          headerActions={
            <span className="ml-auto flex gap-3 text-xs">
              <Link href={downloadUrl(src)} download>Download</Link>
              <Link href={src} external>Open in a new tab</Link>
            </span>
          }
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
    queryKey: ticketKeys.attachmentText(src),
    queryFn: async () => {
      const res = await fetch(src);
      if (!res.ok) throw new Error(res.status === 404 ? 'This file is no longer attached.' : `HTTP ${res.status}`);
      return res.text();
    },
    staleTime: Infinity,
  });
  if (q.isPending) return <Skeleton lines={4} label="Loading the file" />;
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
        className="inline-flex h-5 max-w-full min-w-0 items-center gap-1 rounded-md bg-muted px-1.5 align-middle font-mono text-meta text-foreground outline-none hover:bg-accent hover:text-link focus-visible:ring-2 focus-visible:ring-ring"
      >
        <FileText aria-hidden className="size-3 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate">{children ?? name}</span>
      </button>
      {open && (
        <Dialog
          onClose={() => setOpen(false)}
          title={name}
          size="xl"
          footer={
            <>
              <span className="mr-auto flex gap-3 self-center text-xs">
                <Link href={downloadUrl(src)} download>Download</Link>
                <Link href={src} external>Open raw</Link>
              </span>
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
