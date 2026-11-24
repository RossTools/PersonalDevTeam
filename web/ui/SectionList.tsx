import type { AriaAttributes, ReactNode, Ref } from 'react';
import { cn } from './lib/utils';

export interface SectionListProps extends AriaAttributes {
  /** `SectionItem`s. */
  children: ReactNode;
  className?: string;
}

/** A list inside a `Section`'s card whose items are its rows (worktrees, remote instances), each divided from the next. */
export function SectionList({ children, className, ...aria }: SectionListProps) {
  return (
    <ul {...aria} data-flush className={cn('flex flex-col divide-y divide-border', className)}>
      {children}
    </ul>
  );
}

export interface SectionItemProps {
  /** The item's name (a string is set in medium weight), or what names it (a branch chip). */
  title: ReactNode;
  /** Before the title: a checkbox that picks the item. */
  leading?: ReactNode;
  /** After the title: its state (`Badge`s, a version). */
  badges?: ReactNode;
  /** On the right of the first line: its buttons. */
  actions?: ReactNode;
  /** Under the first line, before `meta`: an `Alert` about it. */
  notice?: ReactNode;
  /** A muted line of facts under it; each child is spaced from the next, wrapping on a narrow screen. */
  meta?: ReactNode;
  /** More under the meta line (a sub-list, a setting of its own). */
  children?: ReactNode;
  className?: string;
  /** The row's `<li>` (to move focus on when it goes away). */
  ref?: Ref<HTMLLIElement>;
}

/** One row of a `SectionList`: name, badges and buttons on a line, then a notice, a meta line and anything more. */
export function SectionItem({ title, leading, badges, actions, notice, meta, children, className, ref }: SectionItemProps) {
  return (
    <li ref={ref} className={cn('flex flex-col gap-1 px-4 py-2.5', className)}>
      <div className="flex flex-wrap items-center gap-2">
        {leading}
        {typeof title === 'string' ? <span className="font-medium">{title}</span> : title}
        {badges}
        {actions != null && <span className="ml-auto flex flex-wrap items-center gap-2">{actions}</span>}
      </div>
      {notice}
      {meta != null && <div className="flex min-w-0 flex-wrap gap-x-3 text-meta text-muted-foreground">{meta}</div>}
      {children}
    </li>
  );
}
