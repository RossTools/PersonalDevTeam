// One layout for every Inbox item (PDT-361): the header (kind, ticket, persona, time, title and links), the line on
// why it waits, its content, and the action bar fixed at the bottom.
import { Fragment, type ReactNode } from 'react';
import { Info } from 'lucide-react';
import { ActionBar, Avatar, Button, Kbd, RelativeTime, Textarea } from '@ui';
import { cn } from '@ui/lib/utils';
import type { InboxItem } from '../api/gen/InboxItem';
import { TicketKey } from '../common/TicketKey';
import { ItemLinks } from './ItemLinks';
import type { ItemAction, ItemSpec } from './itemView';
import { KindChip } from './kinds';
import { longTitle } from './model';

/** The kind, the ticket's key and title, the persona and the age; then the title and the links row. */
export function ItemHeader({ item, spec, className }: { item: InboxItem; spec: ItemSpec; className?: string }) {
  const parts: ReactNode[] = [
    item.ticketKey ? (
      <span key="t" className="flex min-w-0 items-center gap-1.5">
        <TicketKey ticketKey={item.ticketKey} />
        {item.ticketTitle && <span className="truncate">{item.ticketTitle}</span>}
      </span>
    ) : null,
    item.personaName ? (
      <span key="p" className="flex items-center gap-1.5"><Avatar name={item.personaName} size="sm" />{item.personaName}</span>
    ) : typeof item.body.workflow === 'string' ? <span key="w">{item.body.workflow}</span> : null,
    <RelativeTime key="a" time={item.createdAt} />,
  ].filter(Boolean);
  return (
    <header className={cn('flex flex-col gap-1.5 border-b border-border', className)}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <KindChip item={item} />
        {parts.map((p, n) => (
          <Fragment key={n}>
            {n > 0 && <span aria-hidden>·</span>}
            {p}
          </Fragment>
        ))}
      </div>
      {/* A long old-style question (the card below has it whole) is clamped, so it shows once. */}
      <h2 className={cn('text-base leading-snug font-semibold break-words', longTitle(item) && 'line-clamp-2')} title={longTitle(item) ? item.title : undefined}>
        {item.title}
      </h2>
      <ItemLinks item={item} extra={spec.links} />
    </header>
  );
}

/** The one line on why the item waits on the user. */
export function Why({ children }: { children: ReactNode }) {
  return (
    <p className="flex items-start gap-2 text-xs text-muted-foreground">
      <Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
      <span className="min-w-0">{children}</span>
    </p>
  );
}

function ActionButton({ a }: { a: ItemAction }) {
  const key = a.kbd ?? (a.primary ? '↵' : null);
  return (
    <Button
      variant={a.primary ? 'primary' : a.danger ? 'danger' : a.end ? 'ghost' : 'secondary'}
      pending={a.pending}
      disabled={a.disabled}
      aria-expanded={a.expanded}
      onClick={a.run}
    >
      {a.title}
      {key && <span aria-hidden className="pointer-coarse:hidden"><Kbd>{key}</Kbd></span>}
    </Button>
  );
}

/**
 * The desktop's item: the header, then (scrolling) the line on why and the content, then the action bar: the note
 * box, the primary action first, the rest, and Dismiss, Stop and Won't do on the right. Take over's outcomes open
 * under it.
 */
export function ItemLayout({ item, spec, note, onNote }: { item: InboxItem; spec: ItemSpec; note: string; onNote: (s: string) => void }) {
  const main = [...spec.actions.filter((a) => a.primary && !a.end), ...spec.actions.filter((a) => !a.primary && !a.end)];
  const end = spec.actions.filter((a) => a.end);
  const hasBar = spec.actions.length > 0 || !!spec.note;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ItemHeader item={item} spec={spec} className="px-gutter pt-4 pb-3" />
      <div className="min-h-0 flex-1 overflow-auto px-gutter py-5">
        <div className="flex max-w-3xl flex-col gap-4">
          <section aria-label={spec.region} className="flex flex-col gap-4">
            {spec.why && <Why>{spec.why}</Why>}
            {spec.content}
          </section>
        </div>
      </div>
      {hasBar && (
        <ActionBar
          aria-label="Your choice"
          note={spec.note && (
            <div className="flex flex-col gap-1">
              <Textarea aria-label={spec.note.label} placeholder={spec.note.placeholder} value={note} rows={1}
                className="min-h-9" onChange={(e) => onNote(e.target.value)} />
              {spec.note.hint && <p className="text-meta text-muted-foreground">{spec.note.hint}</p>}
            </div>
          )}
          hint={spec.hint && <span aria-live="polite">{spec.hint}</span>}
          end={end.length > 0 && end.map((a) => <ActionButton key={a.id} a={a} />)}
          below={spec.takeOver?.open && (
            <div role="group" aria-label="Pick the outcome" className="flex flex-wrap items-center gap-2">
              <span className="text-sm text-muted-foreground">The step ends with:</span>
              {spec.takeOver.outcomes.map((o) => (
                <Button key={o.id} variant="secondary" onClick={() => spec.takeOver!.pick(o.id)}>{o.title}</Button>
              ))}
            </div>
          )}
        >
          {main.map((a) => <ActionButton key={a.id} a={a} />)}
        </ActionBar>
      )}
    </div>
  );
}
