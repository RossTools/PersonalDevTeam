import type { AriaAttributes, ReactNode } from 'react';
import { Link as LinkPrimitive, type PressEvent } from 'react-aria-components';
import { ExternalLinkIcon } from 'lucide-react';
import { cn } from 'cn';

export interface LinkProps extends AriaAttributes {
  children: ReactNode;
  href?: string;
  /** Opens in a new tab (target=_blank, rel=noreferrer) and shows the external-link icon. */
  external?: boolean;
  /** Called on press; with no `href` the link is an in-app action. */
  onClick?: (e: PressEvent) => void;
  title?: string;
  className?: string;
}

/** Text link: internal (href or onClick) or `external`. */
export function Link({ children, href, external = false, onClick, className, ...rest }: LinkProps) {
  return (
    <LinkPrimitive
      {...rest}
      href={href}
      onPress={onClick}
      {...(external ? { target: '_blank', rel: 'noreferrer' } : {})}
      className={cn(
        'inline-flex items-center gap-0.5 rounded-sm text-primary underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring data-disabled:opacity-50 cursor-pointer',
        className,
      )}
    >
      {children}
      {external && <ExternalLinkIcon aria-hidden className="size-3 shrink-0" />}
    </LinkPrimitive>
  );
}
