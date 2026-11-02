import { cn } from '@ui/lib/utils';
import { Chip } from '@ui/Chip';

export interface TicketKeyProps {
  /** The ticket's key, `PDT-42`. */
  ticketKey: string;
  /** Makes the chip a button (opens the ticket). */
  onClick?: () => void;
  /** Makes the chip a link (same tab unless it is an http(s) URL). */
  href?: string;
  /** Tooltip, e.g. the ticket's title. */
  title?: string;
  className?: string;
}

/** A ticket key in a mono chip; a button with `onClick`, a link with `href`, plain text otherwise. */
export function TicketKey({ ticketKey, onClick, href, title, className }: TicketKeyProps) {
  if (href) {
    return (
      <Chip href={href} title={title} aria-label={title ? `${ticketKey}: ${title}` : undefined} className={className}>
        {ticketKey}
      </Chip>
    );
  }
  if (onClick) {
    return (
      <button
        type="button"
        title={title}
        onClick={onClick}
        className={cn(
          'inline-flex max-w-full min-w-0 shrink-0 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring hover:[&>span]:bg-accent hover:[&>span]:text-link',
          className,
        )}
      >
        <Chip>{ticketKey}</Chip>
      </button>
    );
  }
  return (
    <Chip title={title} className={className}>
      {ticketKey}
    </Chip>
  );
}
