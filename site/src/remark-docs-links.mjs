// Makes the repository's Markdown read well on the site:
// - drops the page's first `# heading` (it is the page title, which Starlight prints itself);
// - points relative links between published docs (`tickets.md#start`, `../SDLC.md`) at their pages on the site,
//   and relative links to anything else in the repository at the file on GitHub.
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { visit } from 'unist-util-visit';
import { docId, isPublished } from './docs-pages.mjs';

const REPO = 'https://github.com/RossTools/PersonalDevTeam/blob/main';
const DOCS_DIR = fileURLToPath(new URL('../../docs', import.meta.url));

/** Rewrites `href`, found in the file at `docsPath` (relative to docs/), or returns it unchanged. */
export function rewriteLink(href, docsPath, base = '/') {
  if (/^([a-z][a-z0-9+.-]*:|#|\/)/i.test(href)) return href;
  const [file, hash] = href.split(/(?=#)/);
  if (!file) return href;
  const target = path.posix.normalize(path.posix.join(path.posix.dirname(docsPath), file));
  if (target.startsWith('../')) return `${REPO}/${path.posix.normalize(target.slice(3))}${hash ?? ''}`;
  if (isPublished(target)) return `${base.replace(/\/?$/, '/')}${docId(target)}/${hash ?? ''}`;
  return `${REPO}/docs/${target}${hash ?? ''}`;
}

export function remarkDocsLinks() {
  const base = process.env.BASE_PATH || '/';
  return (tree, file) => {
    const filePath = file.path ?? file.history?.[0];
    if (!filePath) return;
    const docsPath = path.relative(DOCS_DIR, filePath).split(path.sep).join('/');
    if (docsPath.startsWith('..')) return;
    const first = tree.children.findIndex((n) => n.type === 'heading');
    if (first !== -1 && tree.children[first].depth === 1) tree.children.splice(first, 1);
    visit(tree, ['link', 'definition'], (node) => {
      node.url = rewriteLink(node.url, docsPath, base);
    });
  };
}
