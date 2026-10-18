import type { ReactNode } from 'react';
import { cn } from 'cn';

const base =
  'inline-flex h-5 max-w-full min-w-0 shrink-0 items-center gap-1 rounded-md bg-muted px-1.5 font-mono text-meta text-foreground [&>svg]:size-3 [&>svg]:shrink-0 [&>svg]:text-muted-foreground';

/**
 * A neutral mono label for keys, branches and labels. With `href` it is a link (http(s) links open in a new tab
 * with `rel=noreferrer` unless `external={false}`).
 */
export function Chip({
  children,
  icon,
  href,
  external,
  title,
  className,
  'aria-label': ariaLabel,
}: {
  children: ReactNode;
  icon?: ReactNode;
  href?: string;
  external?: boolean;
  title?: string;
  className?: string;
  'aria-label'?: string;
}) {
  const content = (
    <>
      {icon}
      <span className="min-w-0 truncate">{children}</span>
    </>
  );
  if (href) {
    const ext = external ?? /^https?:\/\//.test(href);
    return (
      <a
        href={href}
        title={title}
        aria-label={ariaLabel}
        target={ext ? '_blank' : undefined}
        rel={ext ? 'noreferrer' : undefined}
        className={cn(
          base,
          'outline-none hover:bg-accent hover:text-primary focus-visible:ring-2 focus-visible:ring-ring',
          className,
        )}
      >
        {content}
      </a>
    );
  }
  return (
    <span title={title} aria-label={ariaLabel} className={cn(base, className)}>
      {content}
    </span>
  );
}
