// @ts-check
import { readdirSync, readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { unified } from '@astrojs/markdown-remark';
import { remarkDocsLinks } from './src/remark-docs-links.mjs';
import { docId } from './src/docs-pages.mjs';

// Starlight's `autogenerate` only sees src/content/docs, and the docs live in the repository's docs/: list them here,
// each labelled with the start of its title ("Host: daemon, desktop tray, …" -> "Host").
const referenceDir = new URL('../docs/reference/', import.meta.url);
const reference = readdirSync(referenceDir)
  .filter((f) => f.endsWith('.md'))
  .sort()
  .map((f) => {
    const title = /^#\s+(.+)$/m.exec(readFileSync(new URL(f, referenceDir), 'utf8'))?.[1] ?? f;
    return { slug: docId(`reference/${f}`), label: title.split(/:| \(|,/)[0].trim() };
  });

// The Pages workflow passes the deployed URL and path (actions/configure-pages); locally the site is served at /.
// With a custom domain (site/public/CNAME) Pages serves it at the domain's root, and configure-pages says so.
const site = process.env.SITE_URL || 'http://localhost:4321';
const base = process.env.BASE_PATH || '/';

export default defineConfig({
  site,
  base,
  trailingSlash: 'ignore',
  markdown: {
    processor: unified({ remarkPlugins: [remarkDocsLinks] }),
  },
  integrations: [
    starlight({
      title: 'PDT',
      description: 'Personal Dev Team: a dev team that lives on your machine.',
      logo: {
        dark: './src/assets/pdt-mark-on-dark.svg',
        light: './src/assets/pdt-mark-on-light.svg',
        replacesTitle: true,
      },
      favicon: '/pdt-favicon-16.svg',
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/RossTools/PersonalDevTeam' }],
      customCss: ['./src/styles/fonts.css', './src/styles/theme.css'],
      components: {
        // Dark unless the reader picked light.
        ThemeProvider: './src/components/ThemeProvider.astro',
      },
      sidebar: [
        { label: 'Overview', link: '/docs/' },
        {
          label: 'Guides',
          items: [
            { label: 'Working in a PDT project', slug: 'docs/agent-guide' },
            { label: 'Run configuration', slug: 'docs/run-config' },
            { label: 'How we work on PDT', slug: 'docs/sdlc' },
          ],
        },
        { label: 'Reference', items: reference },
      ],
    }),
  ],
});
