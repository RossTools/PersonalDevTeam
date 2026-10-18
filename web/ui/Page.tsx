import { useId, type ReactNode } from 'react';
import { cn } from './lib/utils';

export interface PageProps {
  title: ReactNode;
  description?: ReactNode;
  /** Top-right of the page (rare: most actions live in their section). */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** A settings-style page: scrolls, max width 3xl, a title and optional description, then `Section`s. */
export function Page({ title, description, actions, children, className }: PageProps) {
  const id = useId();
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <article aria-labelledby={id} className={cn('flex max-w-3xl flex-col gap-6 p-4 text-sm sm:p-6', className)}>
        <header className="flex flex-wrap items-start gap-x-4 gap-y-2">
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <h2 id={id} className="text-base font-semibold">{title}</h2>
            {description != null && <p className="text-xs text-muted-foreground">{description}</p>}
          </div>
          {actions != null && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
        {children}
      </article>
    </div>
  );
}

export interface SectionProps {
  /** The heading: a small uppercase label. */
  label: ReactNode;
  description?: ReactNode;
  /** Next to the label (a section's single Save, an "Add" button). */
  actions?: ReactNode;
  /** A Danger zone: red label and a soft red border. */
  danger?: boolean;
  children?: ReactNode;
  className?: string;
}

/** A group on a `Page`: label, optional description, then rows. */
export function Section({ label, description, actions, danger = false, children, className }: SectionProps) {
  const id = useId();
  return (
    <section
      aria-labelledby={id}
      className={cn('flex flex-col gap-3', danger && 'rounded-md border border-destructive/30 p-3', className)}
    >
      <div className="flex flex-col gap-1">
        <div className="flex min-h-control-sm items-center gap-2">
          <h3
            id={id}
            className={cn(
              'flex-1 text-2xs font-medium tracking-wider uppercase',
              danger ? 'text-destructive' : 'text-muted-foreground',
            )}
          >
            {label}
          </h3>
          {actions != null && <div className="flex items-center gap-2">{actions}</div>}
        </div>
        {description != null && <p className="text-xs text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  );
}

export interface SettingsRowProps {
  label: ReactNode;
  description?: ReactNode;
  /** The id of the control, so the label is its `<label>`; without it the row is a labelled group. */
  htmlFor?: string;
  /** The control, on the right (below the label on a phone). */
  children: ReactNode;
  className?: string;
}

/** One setting: label and description on the left, the control on the right; stacked on a phone. */
export function SettingsRow({ label, description, htmlFor, children, className }: SettingsRowProps) {
  const labelId = useId();
  const descId = useId();
  const labelClass = 'text-sm text-foreground';
  return (
    <div
      role="group"
      aria-labelledby={labelId}
      aria-describedby={description != null ? descId : undefined}
      className={cn('flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between sm:gap-6', className)}
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        {htmlFor ? (
          <label id={labelId} htmlFor={htmlFor} className={labelClass}>{label}</label>
        ) : (
          <span id={labelId} className={labelClass}>{label}</span>
        )}
        {description != null && <p id={descId} className="text-xs text-muted-foreground">{description}</p>}
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">{children}</div>
    </div>
  );
}
