import { readFile } from 'node:fs/promises';
import { defineCollection } from 'astro:content';
import { glob, type Loader } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';
import { DOCS, docId } from './docs-pages.mjs';
import { docDescription } from './doc-description.mjs';

// The docs are read in place from the repository's docs/ folder: no copy step, so an edit there reaches the site on
// the next build. They are plain Markdown without front matter, so each page's title is its first `# heading`
// (remark-docs-links.mjs drops that heading from the body, as Starlight prints the title itself) and its description
// (search results, link previews) its first paragraph.
const repoDocs = glob({ base: '../docs', pattern: DOCS, generateId: ({ entry }) => docId(entry) });

const docsLoader: Loader = {
  name: 'pdt-docs',
  load: (context) =>
    repoDocs.load({
      ...context,
      parseData: async (entry) => {
        if (!entry.data.title && entry.filePath) {
          const text = await readFile(entry.filePath, 'utf8');
          const heading = /^#\s+(.+)$/m.exec(text);
          entry.data = {
            ...entry.data,
            title: heading ? heading[1].trim() : entry.id,
            description: docDescription(text) || undefined,
          };
        }
        return context.parseData(entry);
      },
    }),
};

export const collections = {
  docs: defineCollection({ loader: docsLoader, schema: docsSchema() }),
};
