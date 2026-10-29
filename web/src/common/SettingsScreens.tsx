import type { ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { EmptyState, List, ListRow, Page, SettingsScreen } from '@ui';
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
  /** Where a section's screen is, above its title on a desktop ("This host", "Projects › PDT"). */
  crumb?: ReactNode;
  /** Above the sections (and the phone's list of them): a load error. */
  notice?: ReactNode;
  sections: SettingsSection[];
  /** Phones: the list of sections while `section` is null (PDT-121). */
  drill?: boolean;
  /** The section on show (its slug, `sectionSlug(label)`); on a desktop, null is the first. */
  section?: string | null;
  onSection?: (slug: string) => void;
}

/**
 * A Settings page with sections, each a screen of its own (PDT-231), reached from the Settings sidebar and by its path.
 * On a desktop the page's own path shows its first section; on a phone it is the list of its sections, iOS-style.
 */
export function SettingsScreens({ title, description, actions, crumb, notice, sections, drill = false, section = null, onSection }: SettingsScreensProps) {
  if (drill && section === null) {
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
  const shown = section === null ? sections[0] : sections.find((s) => sectionSlug(s.label) === section);
  if (!shown) {
    return (
      <Page title={title}>
        {notice}
        <EmptyState>This page has no section “{section}”.</EmptyState>
      </Page>
    );
  }
  // A phone's top bar already names the page and section.
  return (
    <SettingsScreen key={shown.label} crumb={drill ? undefined : crumb} lead={shown.label} notice={notice}>
      {shown.node}
    </SettingsScreen>
  );
}

/** A section's heading from its slug, among the labels a page may have; null when none matches. */
export function sectionLabel(labels: readonly string[], slug: string): string | null {
  return labels.find((l) => sectionSlug(l) === slug) ?? null;
}
