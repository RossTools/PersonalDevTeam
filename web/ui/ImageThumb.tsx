import { ImageOff, X } from 'lucide-react';
import { cn } from './lib/utils';
import { Spinner } from './Spinner';

export interface ImageThumbProps {
  /** The image (an object URL while composing, the attachment's URL once sent); none: a placeholder. */
  src?: string;
  alt: string;
  /** Shows a remove button (composing). */
  onRemove?: () => void;
  /** Still uploading. */
  pending?: boolean;
  /** It could not be attached: why (shown on hover, with an error border). */
  error?: string;
  /** Opens the full image in a new tab. */
  href?: string;
  size?: 'sm' | 'md';
  className?: string;
}

/** A square image preview: an attachment in the composer (removable) or on a sent message (opens full size). */
export function ImageThumb({ src, alt, onRemove, pending = false, error, href, size = 'md', className }: ImageThumbProps) {
  const box = size === 'sm' ? 'size-12' : 'size-20';
  const img = src ? (
    <img src={src} alt={alt} className="size-full object-cover" draggable={false} />
  ) : (
    <span role="img" aria-label={alt} className="flex size-full items-center justify-center text-muted-foreground">
      <ImageOff aria-hidden className="size-4" />
    </span>
  );
  return (
    <div
      title={error ?? alt}
      className={cn(
        'relative shrink-0 overflow-hidden rounded-md border bg-muted',
        error ? 'border-destructive' : 'border-border',
        box,
        className,
      )}
    >
      {href ? (
        <a href={href} target="_blank" rel="noopener noreferrer" className="block size-full outline-none focus-visible:ring-2 focus-visible:ring-ring">
          {img}
        </a>
      ) : (
        img
      )}
      {pending && (
        <span className="absolute inset-0 flex items-center justify-center bg-backdrop">
          <Spinner label="Uploading" className="size-4" />
        </span>
      )}
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove ${alt}`}
          onClick={onRemove}
          className="absolute top-0.5 right-0.5 flex size-5 items-center justify-center rounded-full bg-card text-foreground shadow-card outline-none focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:size-7"
        >
          <X aria-hidden className="size-3" />
        </button>
      )}
    </div>
  );
}
