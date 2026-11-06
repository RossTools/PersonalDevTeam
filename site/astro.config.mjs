// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { unified } from '@astrojs/markdown-remark';
import { remarkDocsLinks } from './src/remark-docs-links.mjs';
import { docId, MOVED, USER_SECTIONS } from './src/docs-pages.mjs';
import { sharedHead } from './src/head.mjs';
import { siteUrl } from './src/site-url.mjs';
import { SITE_NAME, TAGLINE } from './src/brand.mjs';

// Starlight's `autogenerate` only sees src/content/docs, and the docs live in the repository's docs/: the user docs
// sections are listed in docs-pages.mjs, each page labelled with its title. Contributing also lists the developer
// guide (docs/dev/) and SDLC.md.
const sections = USER_SECTIONS.map((s) => ({
  label: s.label,
  items: [...s.pages.map((p) => `user/${s.dir}/${p}.md`), ...(s.more ?? [])].map((f) => ({ slug: docId(f) })),
}));

// The Pages workflow passes the deployed URL and path (actions/configure-pages); locally the site is served at /.
// With a custom domain Pages serves it at the domain's root, and configure-pages says so. siteUrl() makes it https://.
const site = siteUrl();
const base = process.env.BASE_PATH || '/';

export default defineConfig({
  site,
  base,
  trailingSlash: 'ignore',
  // Pages that moved keep their old addresses working (Astro puts a redirect's source under the base, not its target).
  redirects: Object.fromEntries(Object.entries(MOVED).map(([from, to]) => [from, `${base.replace(/\/$/, '')}${to}`])),
  markdown: {
    processor: unified({ remarkPlugins: [remarkDocsLinks] }),
  },
  integrations: [
    starlight({
      // Titles read "Page · PDT (Personal Dev Team)", as on the landing frame's pages (brand.mjs `pageTitle`).
      title: SITE_NAME,
      titleDelimiter: '·',
      description: `${SITE_NAME}: ${TAGLINE}.`,
      head: sharedHead(site, base),
      // The 404 page is src/pages/404.astro, in the landing frame.
      disable404Route: true,
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
        ...sections,
        {
          label: 'Guides',
          items: [
            { label: 'Working in a PDT project', slug: 'docs/agent-guide' },
          ],
        },
        // The Plugins pages (src/pages/plugins/) stand outside the docs, like the landing page.
        { label: 'Plugins', link: '/plugins/' },
      ],
    }),
  ],
});
