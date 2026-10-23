import { Fragment, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { EmptyState, List, ListRow, Page } from '@ui';
import { sectionSlug } from '../shell/route';

/** One section of a Settings page: its heading (the same as its `Section`'s label) and the section itself. */
export interface SettingsSection {
  label: string;
  node: ReactNode;
}

export interface SettingsScreensProps {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  /** Above the sections (and the phone's list of them): a load error. */
  notice?: ReactNode;
  sections: SettingsSection[];
  /** Phones: a screen per section (PDT-121), the list of them while `section` is null. */
  drill?: boolean;
  /** The section on show (its slug, `sectionSlug(label)`), on a phone. */
  section?: string | null;
  onSection?: (slug: string) => void;
}

/**
 * A Settings page with sections. On a desktop, one page of them all (a section in the path is scrolled to). On a phone,
 * iOS-style: the page is a list of its sections, and each section is a screen of its own.
 */
export function SettingsScreens({ title, description, actions, notice, sections, drill = false, section = null, onSection }: SettingsScreensProps) {
  if (!drill) {
    return (
      <Page title={title} description={description} actions={actions}>
        {notice}
        {sections.map((s) => <Fragment key={s.label}>{s.node}</Fragment>)}
      </Page>
    );
  }
  if (section === null) {
    return (
      <Page title={title} description={description} actions={actions}>
        {notice}
        <List aria-label="Sections" onAction={(id) => onSection?.(id)} className="-mx-1">
          {sections.map((s) => (
            <ListRow key={s.label} id={sectionSlug(s.label)} title={s.label}
              trailing={<ChevronRight aria-hidden className="size-4 text-muted-foreground" />} />
          ))}
        </List>
      </Page>
    );
  }
  const shown = sections.find((s) => sectionSlug(s.label) === section);
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="flex flex-col gap-4 p-4 text-sm">
        {notice}
        {shown ? shown.node : <EmptyState>This page has no section “{section}”.</EmptyState>}
      </div>
    </div>
  );
}

/** A section's heading from its slug, among the labels a page may have; null when none matches. */
export function sectionLabel(labels: readonly string[], slug: string): string | null {
  return labels.find((l) => sectionSlug(l) === slug) ?? null;
}
