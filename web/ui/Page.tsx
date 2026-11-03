import { createContext, useContext, useId, type ReactNode } from 'react';
import { cn } from './lib/utils';

export interface PageProps {
  title: ReactNode;
  description?: ReactNode;
  /** Top-right of the page (rare: most actions live in their section). */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

/**
 * A settings page: scrolls, max width 3xl, a title and optional description, then `Section` cards. The settings
 * design language (web/ui/README.md "Settings pages") says what goes where.
 */
export function Page({ title, description, actions, children, className }: PageProps) {
  const id = useId();
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <article aria-labelledby={id} className={cn('flex max-w-3xl flex-col gap-8 p-gutter text-sm', className)}>
        <header className="flex flex-wrap items-start gap-x-4 gap-y-2">
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <h2 id={id} className="text-heading font-semibold">{title}</h2>
            {description != null && <p className="text-xs text-muted-foreground">{description}</p>}
          </div>
          {actions != null && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
        {children}
      </article>
    </div>
  );
}

/** The label of the section a `SettingsScreen` is about: that `Section`'s heading is the screen's title. */
const ScreenLead = createContext<string | null>(null);

export interface SettingsScreenProps {
  /** Above the title: where the screen is ("This host", "Projects › PDT"). */
  crumb?: ReactNode;
  /** The label of the `Section` the screen shows: its heading and description become the screen's title. */
  lead: string;
  /** Above the sections: a load error, or a banner. */
  notice?: ReactNode;
  /** The `Section` labelled `lead`, and any more cards under their own small headings. */
  children: ReactNode;
  className?: string;
}

/**
 * One section of a settings page on a screen of its own (PDT-231): a breadcrumb, the section's heading and description
 * as the screen's title, then its card(s) of rows. Scrolls, max width 3xl, like `Page`.
 */
export function SettingsScreen({ crumb, lead, notice, children, className }: SettingsScreenProps) {
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <article aria-label={lead} className={cn('flex max-w-3xl flex-col gap-6 p-gutter text-sm', className)}>
        {crumb != null && <nav aria-label="Breadcrumb" className="-mb-5 text-xs text-muted-foreground">{crumb}</nav>}
        {notice}
        <ScreenLead.Provider value={lead}>{children}</ScreenLead.Provider>
      </article>
    </div>
  );
}

export interface SectionProps {
  /** The heading above the card. */
  label: ReactNode;
  description?: ReactNode;
  /**
   * Next to the heading: actions on the whole section that change nothing (Check for updates, Run checks again).
   * Secondary buttons; never a destructive one.
   */
  actions?: ReactNode;
  /** Between the heading and the card: a banner about the whole section ("Restart PDT to apply"). */
  notice?: ReactNode;
  /** Bottom of the card, right-aligned: a form section's Save (with its "Saved" note). */
  footer?: ReactNode;
  /**
   * For a form's `footer`: whether something changed. On a phone the footer is then a Save bar that sticks to the
   * bottom of the screen, shown only while something changed.
   */
  dirty?: boolean;
  /** The Danger zone: red heading and border. The only place for page-wide destructive actions. */
  danger?: boolean;
  /**
   * Rows (`SettingsRow`), or any block; each child is padded and divided from the next. A child with `data-flush`
   * pads itself (a list whose items are the rows).
   */
  children?: ReactNode;
  className?: string;
}

/** A group on a `Page`: heading and description, then a card of rows split by lines, then an optional footer. */
export function Section({ label, description, actions, notice, footer, dirty, danger = false, children, className }: SectionProps) {
  const id = useId();
  // The section a SettingsScreen is about: its heading is the screen's title.
  const lead = useContext(ScreenLead) === label;
  const Heading = lead ? 'h2' : 'h3';
  return (
    <section aria-labelledby={id} className={cn('flex flex-col', lead ? 'gap-4' : 'gap-2', className)}>
      <div className="flex flex-wrap items-end gap-x-4 gap-y-2 px-0.5">
        <div className={cn('flex min-w-0 flex-1 flex-col', lead ? 'gap-1' : 'gap-0.5')}>
          <Heading id={id} className={cn(lead ? 'text-heading font-semibold' : 'text-sm font-semibold', danger ? 'text-destructive' : 'text-foreground')}>
            {label}
          </Heading>
          {description != null && <p className={cn(lead ? 'text-sm' : 'text-xs', 'text-muted-foreground')}>{description}</p>}
        </div>
        {actions != null && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {notice}
      <div
        data-slot="section-card"
        className={cn(
          // clip, not hidden: a hidden overflow would be the sticky Save bar's scroller.
          'flex flex-col overflow-clip rounded-lg border bg-card shadow-card',
          danger ? 'border-destructive/40' : 'border-border',
        )}
      >
        {children != null && (
          <div className={cn('contents divide-y [&>*:not([data-flush])]:px-4 [&>*:not([data-flush])]:py-3', danger ? 'divide-destructive/20' : 'divide-border')}>
            {children}
          </div>
        )}
        {footer != null && (
          <div
            data-slot="section-footer"
            data-dirty={dirty === undefined ? undefined : String(dirty)}
            className={cn(
              'flex flex-wrap items-center justify-end gap-2 bg-muted/40 px-4 py-2',
              children != null && (danger ? 'border-t border-destructive/20' : 'border-t border-border'),
              dirty !== undefined && 'max-md:sticky max-md:bottom-0 max-md:bg-card max-md:py-3 max-md:pb-[max(0.75rem,env(safe-area-inset-bottom))]',
              dirty === false && 'max-md:hidden',
            )}
          >
            {footer}
          </div>
        )}
      </div>
    </section>
  );
}

export interface SettingsRowProps {
  label: ReactNode;
  description?: ReactNode;
  /** The id of the control, so the label is its `<label>`; without it the row is a labelled group. */
  htmlFor?: string;
  /** Label above a full-width control (text areas, file views, folder fields) instead of beside it. */
  stacked?: boolean;
  /** The control, on the right (below the label on a phone, or always when `stacked`). */
  children: ReactNode;
  className?: string;
}

/** On a phone (the row stacked) text fields and selects take the row's width, whatever width they ask for. */
const phoneFields = 'max-sm:*:data-[slot=input]:grow max-sm:*:data-[slot=input]:basis-32 max-sm:*:data-[slot=select]:grow max-sm:*:data-[slot=select]:basis-32';

/** One setting: label and description on the left, the control on the right; stacked on a phone. */
export function SettingsRow({ label, description, htmlFor, stacked = false, children, className }: SettingsRowProps) {
  const labelId = useId();
  const descId = useId();
  const labelClass = 'text-sm font-medium text-foreground';
  return (
    <div
      role="group"
      aria-labelledby={labelId}
      aria-describedby={description != null ? descId : undefined}
      className={cn(
        'flex flex-col gap-2',
        !stacked && 'sm:flex-row sm:items-center sm:justify-between sm:gap-6',
        className,
      )}
    >
      <div className={cn('flex min-w-0 flex-col gap-0.5', !stacked && 'sm:max-w-[55%]')}>
        {htmlFor ? (
          <label id={labelId} htmlFor={htmlFor} className={labelClass}>{label}</label>
        ) : (
          <span id={labelId} className={labelClass}>{label}</span>
        )}
        {description != null && <div id={descId} className="text-xs text-muted-foreground">{description}</div>}
      </div>
      <div className={cn('flex min-w-0 flex-wrap items-center gap-2', stacked ? 'flex-col items-stretch' : 'sm:justify-end', phoneFields)}>
        {children}
      </div>
    </div>
  );
}
