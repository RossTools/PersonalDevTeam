// Which of the repository's docs/ the site publishes, and where each lands. Shared by the content collection
// (content.config.ts) and the link rewriting (remark-docs-links.mjs). superpowers/, prompts/ and ROADMAP.md stay out.

/** Globs under docs/. */
export const DOCS = ['agent-guide.md', 'run-config.md', 'SDLC.md', 'reference/*.md'];

/** The collection id (and URL path) of a file under docs/: `reference/tickets.md` -> `docs/reference/tickets`. */
export function docId(entry) {
  return `docs/${entry.replace(/\.md$/, '').toLowerCase()}`;
}

/** Whether a path relative to docs/ (`reference/tickets.md`) is one the site publishes. */
export function isPublished(path) {
  return (
    ['agent-guide.md', 'run-config.md', 'SDLC.md'].includes(path) ||
    /^reference\/[^/]+\.md$/.test(path)
  );
}
