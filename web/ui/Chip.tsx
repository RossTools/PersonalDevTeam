import type { HTMLAttributes, ReactNode, Ref } from 'react';
import { cn } from '@ui/lib/utils';

const base =
  'inline-flex h-5 max-w-full min-w-0 shrink-0 items-center gap-1 rounded-md bg-muted px-1.5 font-mono text-meta text-foreground [&>svg]:size-3 [&>svg]:shrink-0 [&>svg]:text-muted-foreground';

/**
 * A neutral mono label for keys, branches and labels. With `href` it is a link (http(s) links open in a new tab
 * with `rel=noreferrer` unless `external={false}`). It forwards its ref and other props (`data-slot`, a hover card's
 * handlers) to the element.
 */
export function Chip({
  children,
  icon,
  href,
  external,
  title,
  className,
  'aria-label': ariaLabel,
  ref,
  ...rest
}: Omit<HTMLAttributes<HTMLElement>, 'children' | 'title' | 'className' | 'aria-label'> & {
  children: ReactNode;
  icon?: ReactNode;
  href?: string;
  external?: boolean;
  title?: string;
  className?: string;
  'aria-label'?: string;
  ref?: Ref<HTMLElement>;
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
        {...rest}
        ref={ref as Ref<HTMLAnchorElement>}
        href={href}
        title={title}
        aria-label={ariaLabel}
        target={ext ? '_blank' : undefined}
        rel={ext ? 'noreferrer' : undefined}
        className={cn(
          base,
          'outline-none hover:bg-accent hover:text-link focus-visible:ring-2 focus-visible:ring-ring',
          className,
        )}
      >
        {content}
      </a>
    );
  }
  return (
    <span {...rest} ref={ref as Ref<HTMLSpanElement>} title={title} aria-label={ariaLabel} className={cn(base, className)}>
      {content}
    </span>
  );
}
