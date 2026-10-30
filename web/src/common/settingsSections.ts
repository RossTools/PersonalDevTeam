// Settings' sections (PDT-231): each is a screen of its own, listed in the Settings sidebar, whose search finds them by
// their names and the labels of the settings on them. A section's label is its `Section` heading; its path is the
// label's slug (`sectionSlug`).
import { sectionSlug } from '../shell/route';

export interface SectionInfo {
  label: string;
  /** The settings on it (row labels and names people look for), which search matches too. */
  terms: readonly string[];
}

/** This host's sections, in sidebar order. */
export const HOST_SECTIONS: readonly SectionInfo[] = [
  { label: 'Updates', terms: ['Version', 'Check for updates', 'Install updates', 'Update channel', 'Release repository', 'GitHub token', 'Nightly'] },
  {
    label: 'Agents',
    terms: [
      'Default agent', 'Default view', 'Ticket permissions', 'Session permissions', 'Agents running at once', 'Triage per author',
      'Triage per repository', 'Credentials', 'Programs', 'Claude Code', 'Codex', 'Devin', 'Program path', 'PDT_CLAUDE_PROGRAM',
      'PDT_CODEX_PROGRAM', 'PDT_DEVIN_PROGRAM', 'ACP agents', 'Add ACP agent', 'Agent Client Protocol',
    ],
  },
  { label: 'Network', terms: ['Port', 'Allowed hosts', 'Restart', 'PDT_PORT', 'PDT_ALLOWED_HOSTS'] },
  { label: 'Remote access', terms: ['Serve on the tailnet', 'Tailscale', 'Owner', 'Status'] },
  { label: 'Runs', terms: ['Ports', 'Max concurrent runs'] },
  { label: 'Projects folder', terms: ['Folder', 'Clone by URL'] },
  { label: 'Appearance', terms: ['Theme', 'Dark', 'Light', 'System'] },
  { label: 'Bug reports', terms: ['Send reports to', 'Report a bug'] },
  { label: 'Checks', terms: ['git', 'tailscale', 'claude', 'codex', 'devin', 'Run checks again'] },
  { label: 'Setup', terms: ['Get started', 'First run', 'Checklist', 'Onboarding'] },
  { label: 'Danger zone', terms: ['Remove the GitHub token'] },
];

/** The host's other pages, each a single screen. */
export const HOST_PAGES = [
  { id: 'notifications', label: 'Notifications', terms: ['Desktop notifications', 'Toasts', 'Updates'] },
  { id: 'plugins', label: 'Plugins', terms: ['Integrations', 'GitHub', 'Jira', 'Buildkite', 'Discord'] },
  { id: 'connections', label: 'All connections', terms: ['Connections'] },
  { id: 'secrets', label: 'Secrets', terms: ['Keychain', 'Tokens'] },
] as const;

/** A project's sections, in sidebar order; the ones a Personal project (no repository) lacks are marked. */
const PROJECT_SECTIONS: readonly (SectionInfo & { repoOnly?: true })[] = [
  { label: 'General', terms: ['Name', 'Key', 'Base branch', 'Repository folder', 'GitHub'] },
  { label: 'Worktrees', terms: ['Remove worktree', 'Review changes'], repoOnly: true },
  { label: 'Runs on this host', terms: ['Repository configuration', 'Host overrides', 'TOML', 'Set up with an agent'], repoOnly: true },
  { label: 'Agent instructions', terms: ['Instructions', 'CLAUDE.md', 'AGENTS.md'] },
  { label: 'Agents', terms: ['Agent credentials', 'Agent permission mode for workflow steps'] },
  { label: 'Review', terms: ['Merge through pull requests'], repoOnly: true },
  {
    label: 'Pipeline and policies',
    terms: ['Who may merge', 'Approval', 'Labels that merge without approval', 'Green CI', 'Running tickets at once', 'Auto-start', 'Quiet period', 'The PM acts without asking'],
  },
  { label: 'Plugins', terms: ['Jira', 'Buildkite', 'Connection'], repoOnly: true },
  { label: 'Danger zone', terms: ['Remove this project'] },
];

/** A project's sections: all of them, or a Personal project's (no worktrees, runs, review or plugins). */
export function projectSections(kind: string): readonly SectionInfo[] {
  return PROJECT_SECTIONS.filter((s) => kind !== 'personal' || !s.repoOnly);
}

/** Every project section label (a phone's top bar title from a path). */
export const PROJECT_SECTION_LABELS = PROJECT_SECTIONS.map((s) => s.label);
export const HOST_SECTION_LABELS = HOST_SECTIONS.map((s) => s.label);

/** Whether search text finds a section: by its label or any of its settings, ignoring case. */
export function sectionMatches(s: { label: string; terms: readonly string[] }, query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || s.label.toLowerCase().includes(q) || s.terms.some((t) => t.toLowerCase().includes(q));
}

/** A section's slug among `sections`, or the first one's when `slug` is null (a page's own path shows its first). */
export function shownSection(sections: readonly SectionInfo[], slug: string | null): SectionInfo | null {
  if (slug === null) return sections[0] ?? null;
  return sections.find((s) => sectionSlug(s.label) === slug) ?? null;
}
