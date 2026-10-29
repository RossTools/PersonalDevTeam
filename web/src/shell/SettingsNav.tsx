import { useEffect, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Plus, Search } from 'lucide-react';
import { Badge, IconButton, Input } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { ProjectKey } from '../common/BranchChip';
import { HOST_PAGES, HOST_SECTIONS, projectSections, sectionMatches, shownSection, type SectionInfo } from '../common/settingsSections';
import { sectionSlug, type SettingsPage } from './route';

const HOST = 'host';

export interface SettingsNavProps {
  projects: Project[];
  /** The page on show (`index` is This host's first section on a desktop). */
  page: SettingsPage;
  /** A phone: no row is marked current on the list screen. */
  phone: boolean;
  onGo: (page: SettingsPage) => void;
  onAddProject: () => void;
  heading: ReactNode;
}

/**
 * The Settings sidebar (PDT-231): a search box, then This host's sections, the host's other pages, and each project's
 * sections, in groups that open and close. Search filters by section names and the settings on them, opening every
 * group with a match.
 */
export function SettingsNav({ projects, page, phone, onGo, onAddProject, heading }: SettingsNavProps) {
  const [query, setQuery] = useState('');
  const searching = query.trim() !== '';
  // Settings → Network saved but not applied yet: a "restart" badge until PDT restarts.
  const host = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const restart = host.data?.network?.restartNeeded ?? false;
  const currentGroup = page.kind === 'project' ? page.key : page.kind === 'host' || page.kind === 'index' ? HOST : null;
  const [open, setOpen] = useState<Set<string>>(() => new Set(currentGroup ? [currentGroup] : []));
  // The group of the screen on show opens (a link, the palette, Back).
  useEffect(() => {
    if (currentGroup) setOpen((o) => (o.has(currentGroup) ? o : new Set([...o, currentGroup])));
  }, [currentGroup]);
  const toggle = (g: string) => setOpen((o) => {
    const next = new Set(o);
    if (!next.delete(g)) next.add(g);
    return next;
  });
  const isOpen = (g: string) => searching || open.has(g);
  // The section marked current: on a desktop a page's own path shows its first section.
  const currentSlug = (sections: readonly SectionInfo[], slug: string | null) => {
    if (phone && slug === null) return null;
    const s = shownSection(sections, slug);
    return s ? sectionSlug(s.label) : null;
  };
  const hostSlug = page.kind === 'host' || (page.kind === 'index' && !phone) ? currentSlug(HOST_SECTIONS, page.kind === 'host' ? page.section : null) : null;
  const hostShown = HOST_SECTIONS.filter((s) => sectionMatches(s, query));
  const pagesShown = HOST_PAGES.filter((p) => sectionMatches(p, query));
  const projectsShown = projects
    .map((p) => {
      const all = projectSections(p.kind);
      // The project's own name finds all its sections.
      const named = sectionMatches({ label: p.name, terms: [p.key] }, query);
      return { p, all, shown: named ? all : all.filter((s) => sectionMatches(s, query)) };
    })
    .filter((x) => !searching || x.shown.length > 0);
  const nothing = searching && hostShown.length === 0 && pagesShown.length === 0 && projectsShown.length === 0;
  return (
    <nav aria-label="Settings sections" className="flex flex-col gap-0.5 px-2 py-1">
      {heading}
      <div className="relative mb-1 px-1">
        <Search aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          aria-label="Search settings"
          type="search"
          placeholder="Search settings"
          className="w-full pl-7"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Escape') setQuery(''); }}
        />
      </div>
      {nothing && <p className="px-3 py-2 text-xs text-muted-foreground">No settings match “{query.trim()}”.</p>}
      {hostShown.length > 0 && (
        <Group label="This host" expanded={isOpen(HOST)} onToggle={() => toggle(HOST)}>
          {hostShown.map((s) => {
            const slug = sectionSlug(s.label);
            return (
              <Item key={slug} current={hostSlug === slug} onPress={() => onGo({ kind: 'host', section: slug })}
                badge={slug === 'network' && restart ? <Badge tone="waiting">restart</Badge> : null}>
                {s.label}
              </Item>
            );
          })}
        </Group>
      )}
      {pagesShown.map((p) => (
        <Item key={p.id} top current={page.kind === p.id} onPress={() => onGo(pageFor(p.id))}>{p.label}</Item>
      ))}
      <h3 className="flex items-center px-2 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase">
        <span className="flex-1">Projects</span>
        <IconButton label="Add project" size="sm" noTooltip className="size-5" onClick={onAddProject}>
          <Plus aria-hidden />
        </IconButton>
      </h3>
      {projectsShown.map(({ p, all, shown }) => {
        const here = page.kind === 'project' && page.key === p.key;
        const slug = here ? currentSlug(all, page.section) : null;
        return (
          <Group key={p.id} label={p.name} trailing={<ProjectKey projectKey={p.key} />} expanded={isOpen(p.key)} onToggle={() => toggle(p.key)}>
            {shown.map((s) => {
              const ss = sectionSlug(s.label);
              return (
                <Item key={ss} current={slug === ss} onPress={() => onGo({ kind: 'project', key: p.key, section: ss })}>{s.label}</Item>
              );
            })}
          </Group>
        );
      })}
      {phone && (
        <Item top onPress={onAddProject}>
          <span className="flex items-center gap-2"><Plus aria-hidden className="size-4 text-muted-foreground" />Add project</span>
        </Item>
      )}
    </nav>
  );
}

function pageFor(id: (typeof HOST_PAGES)[number]['id']): SettingsPage {
  return id === 'plugins' ? { kind: 'plugins', id: null } : { kind: id };
}

function Group({ label, trailing, expanded, onToggle, children }: {
  label: string;
  trailing?: ReactNode;
  expanded: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return (
    <div className="flex flex-col gap-0.5">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={onToggle}
        className="flex min-h-7 w-full items-center gap-1.5 rounded-md px-2 text-left text-sm font-semibold hover:bg-accent pointer-coarse:min-h-11"
      >
        <Chevron aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {trailing}
      </button>
      {expanded && <div role="group" aria-label={label} className="flex flex-col gap-0.5">{children}</div>}
    </div>
  );
}

function Item({ current = false, top = false, badge, onPress, children }: {
  current?: boolean;
  /** A page of its own at the top level, not a group's section. */
  top?: boolean;
  badge?: ReactNode;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-current={current ? 'page' : undefined}
      onClick={onPress}
      className={cn(
        'flex min-h-7 w-full items-center gap-2 rounded-md pr-2 text-left text-sm hover:bg-accent pointer-coarse:min-h-11',
        top ? 'pl-2 font-semibold' : 'pl-7',
        current && 'bg-accent font-medium',
      )}
    >
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {/* A space, so the name reads "Network restart". */}
      {badge && <>{' '}{badge}</>}
    </button>
  );
}
