// Which of the repository's docs/ the site publishes, and where each lands. Shared by the content collection
// (content.config.ts), the link rewriting (remark-docs-links.mjs), the sidebar (astro.config.mjs) and the docs home.
// superpowers/, prompts/, ROADMAP.md and the READMEs in docs/user/ (notes for whoever writes the docs) stay out.

/** Globs under docs/. */
export const DOCS = ['agent-guide.md', 'run-config.md', 'SDLC.md', 'reference/*.md', 'user/*/*.md', '!user/**/README.md'];

/**
 * The user docs (docs/user/<section>/<page>.md), in sidebar order. Each section's `summary` is its line on the docs
 * home. A page's sidebar label is its title (its first `# heading`).
 */
export const USER_SECTIONS = [
  {
    dir: 'getting-started',
    label: 'Getting started',
    summary: 'Install PDT, connect an agent and GitHub, and take a first ticket all the way to a merged pull request.',
    pages: ['what-is-pdt', 'install', 'first-run', 'connect', 'first-project', 'first-ticket', 'next-steps'],
  },
  {
    dir: 'using-pdt',
    label: 'Using PDT',
    summary: 'Tickets and the board, workspaces and sessions, reviewing and merging, and the Inbox.',
    pages: ['tickets-and-the-board', 'workspaces-and-sessions', 'review-and-merge', 'the-inbox'],
  },
  {
    dir: 'automation',
    label: 'Automation',
    summary: 'Workflows, the PM, personas and approvals, plugins, how Run and Test start your project, and outside contributions.',
    pages: ['workflows', 'the-pm', 'personas-and-approvals', 'plugins', 'run-configuration', 'outside-contributions'],
  },
  {
    dir: 'your-machine',
    label: 'Your machine',
    summary: 'Updates, remote access from a phone or another computer, GitHub, secrets, settings and where your data lives.',
    pages: ['install-and-updates', 'remote-access', 'github', 'secrets', 'settings', 'your-data'],
  },
  {
    dir: 'troubleshooting',
    label: 'Troubleshooting & FAQ',
    summary: 'What to check when something goes wrong, and answers to common questions.',
    pages: ['troubleshooting', 'faq'],
  },
  {
    dir: 'contributing',
    label: 'Contributing',
    summary: 'Report a bug, suggest a feature, or work on PDT itself.',
    pages: ['contributing'],
  },
];

/**
 * The collection id (and URL path) of a file under docs/: `reference/tickets.md` -> `docs/reference/tickets`. The user
 * docs are the site's main docs, so they drop `user/`: `user/getting-started/install.md` -> `docs/getting-started/install`.
 */
export function docId(entry) {
  return `docs/${entry.replace(/^user\//, '').replace(/\.md$/, '').toLowerCase()}`;
}

/** Whether a path relative to docs/ (`reference/tickets.md`) is one the site publishes. */
export function isPublished(path) {
  return (
    ['agent-guide.md', 'run-config.md', 'SDLC.md'].includes(path) ||
    /^reference\/[^/]+\.md$/.test(path) ||
    (/^user\/[^/]+\/[^/]+\.md$/.test(path) && !path.endsWith('/README.md'))
  );
}
