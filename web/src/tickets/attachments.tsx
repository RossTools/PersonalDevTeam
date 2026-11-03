import { useCallback, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Paperclip, Trash2, X } from 'lucide-react';
import { Button, EmptyState, IconButton, useConfirm } from '@ui';
import { api } from '../api/client';
import type { TicketAttachment } from '../api/gen/TicketAttachment';
import { AttachmentImage, AttachmentLink } from '../common/AttachmentViews';
import { actionError, runAction } from '../common/useAction';
import { POLL } from '../common/poll';
import { OverviewSection } from './TicketParts';

/** What the file pickers offer: the images and text files a ticket takes (pdtd decides by the bytes). */
export const ATTACH_ACCEPT = 'image/png,image/jpeg,image/gif,image/webp,text/*,.txt,.log,.md,.json,.csv,.yaml,.yml,.toml';

const attachmentKeys = { of: (ticketId: string) => ['ticketAttachments', ticketId] as const };

export function useTicketAttachments(ticketId: string) {
  return useQuery({
    queryKey: attachmentKeys.of(ticketId),
    queryFn: () => api.ticketAttachments(ticketId),
    refetchInterval: POLL.live,
  });
}

/** Markdown that shows an attachment: an image inline, a text file as a link (as pdtd writes it). */
export function attachmentMarkdown(a: TicketAttachment): string {
  const name = a.name.replace(/[[\]]/g, '');
  const url = api.ticketAttachmentUrl(a.ticketId, a.id);
  return a.mediaType.startsWith('image/') ? `![${name}](${url})` : `[${name}](${url})`;
}

/** One upload in flight, or one that failed (it stays, with why, until dismissed: PDT-135). */
export interface Upload {
  key: string;
  name: string;
  /** 0–1. */
  progress: number;
  error?: string;
}

/** A pasted image has no useful name ("image.png"): name it by when it was pasted. */
function nameOf(f: File): string {
  if (f.name && f.name !== 'image.png') return f.name;
  const ext = f.type.split('/')[1] ?? 'png';
  return `screenshot-${new Date().toISOString().slice(0, 19).replace(/[T:]/g, '-')}.${ext}`;
}

/**
 * Uploads files to a ticket, tracking each one's progress. `add` resolves with the attachments that made it (the
 * others stay in `uploads` with their error).
 */
export function useAttachmentUploads(ticketId: string) {
  const qc = useQueryClient();
  const [uploads, setUploads] = useState<Upload[]>([]);
  const seq = useRef(0);
  const add = useCallback(async (files: File[]): Promise<TicketAttachment[]> => {
    const done = await Promise.all(files.map(async (f) => {
      const key = `${Date.now()}-${seq.current++}`;
      const name = nameOf(f);
      setUploads((u) => [...u, { key, name, progress: 0 }]);
      try {
        const a = await api.uploadTicketAttachment(ticketId, f, name, (progress) =>
          setUploads((u) => u.map((x) => (x.key === key ? { ...x, progress } : x))));
        setUploads((u) => u.filter((x) => x.key !== key));
        return a;
      } catch (e) {
        setUploads((u) => u.map((x) => (x.key === key ? { ...x, error: actionError(e) } : x)));
        return null;
      }
    }));
    void qc.invalidateQueries({ queryKey: attachmentKeys.of(ticketId) });
    return done.filter((a): a is TicketAttachment => a !== null);
  }, [qc, ticketId]);
  const dismiss = useCallback((key: string) => setUploads((u) => u.filter((x) => x.key !== key)), []);
  return { uploads, add, dismiss };
}

/** Uploads in flight (with their progress) and failed ones (with why and a dismiss button). */
export function UploadList({ uploads, onDismiss }: { uploads: Upload[]; onDismiss: (key: string) => void }) {
  if (uploads.length === 0) return null;
  return (
    <ul aria-label="Uploads" className="flex flex-col gap-1 text-xs">
      {uploads.map((u) => (
        <li key={u.key} className="flex min-w-0 items-center gap-2">
          <Paperclip aria-hidden className="size-3 shrink-0 text-muted-foreground" />
          <span className="min-w-0 truncate">{u.name}</span>
          {u.error ? (
            <>
              <span role="alert" className="min-w-0 flex-1 truncate text-destructive" title={u.error}>{u.error}</span>
              <IconButton label={`Dismiss ${u.name}`} size="sm" variant="ghost" onClick={() => onDismiss(u.key)}>
                <X aria-hidden />
              </IconButton>
            </>
          ) : (
            <progress
              aria-label={`Uploading ${u.name}`}
              value={u.progress}
              max={1}
              className="h-1.5 w-24 shrink-0 accent-primary"
            />
          )}
        </li>
      ))}
    </ul>
  );
}

/** A hidden file picker and the button that opens it. */
export function AttachButton({ onFiles, label = 'Attach' }: { onFiles: (files: File[]) => void; label?: string }) {
  const picker = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={picker}
        type="file"
        accept={ATTACH_ACCEPT}
        multiple
        hidden
        aria-hidden
        tabIndex={-1}
        onChange={(e) => {
          onFiles([...(e.target.files ?? [])]);
          e.target.value = '';
        }}
      />
      <Button variant="ghost" size="sm" compact onClick={() => picker.current?.click()}>
        <Paperclip aria-hidden /> {label}
      </Button>
    </>
  );
}

const sizeText = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.round(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);

/** The ticket's attachments: images as thumbnails (full size on click), text files as chips, each removable. */
export function TicketAttachments({ ticketId }: { ticketId: string }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const q = useTicketAttachments(ticketId);
  const { uploads, add, dismiss } = useAttachmentUploads(ticketId);
  const list = q.data ?? [];
  const remove = (a: TicketAttachment) => void runAction(async () => {
    const { ok } = await confirm({
      title: `Remove ${a.name}?`,
      body: 'Comments and the description that show it will show a broken link.',
      action: 'Remove attachment',
      danger: true,
    });
    if (!ok) return;
    await api.deleteTicketAttachment(ticketId, a.id);
    void qc.invalidateQueries({ queryKey: attachmentKeys.of(ticketId) });
  });
  return (
    <OverviewSection title="Attachments" actions={<AttachButton onFiles={(f) => void add(f)} />}>
      {list.length === 0 && uploads.length === 0 && (
        <EmptyState size="inline">None. Paste or drop screenshots and logs into a comment or the description.</EmptyState>
      )}
      {list.length > 0 && (
        <ul className="flex flex-col gap-2">
          {list.map((a) => {
            const src = api.ticketAttachmentUrl(ticketId, a.id);
            return (
              <li key={a.id} className="flex min-w-0 items-center gap-2 text-xs">
                <div className="min-w-0 flex-1">
                  {a.mediaType.startsWith('image/') ? (
                    <div className="w-24 [&_button]:my-0 [&_img]:max-h-16"><AttachmentImage src={src} alt={a.name} /></div>
                  ) : (
                    <AttachmentLink src={src} name={a.name} />
                  )}
                  <p className="mt-0.5 truncate text-meta text-muted-foreground" title={a.name}>
                    {a.mediaType.startsWith('image/') && `${a.name} · `}{sizeText(Number(a.size))}
                  </p>
                </div>
                <IconButton label={`Remove ${a.name}`} size="sm" variant="ghost" onClick={() => remove(a)}>
                  <Trash2 aria-hidden />
                </IconButton>
              </li>
            );
          })}
        </ul>
      )}
      <UploadList uploads={uploads} onDismiss={dismiss} />
      {q.isError && <p role="alert" className="text-xs text-destructive">{actionError(q.error)}</p>}
    </OverviewSection>
  );
}
