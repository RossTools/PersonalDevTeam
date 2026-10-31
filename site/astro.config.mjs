// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { unified } from '@astrojs/markdown-remark';
import { remarkDocsLinks } from './src/remark-docs-links.mjs';
import { docId, USER_SECTIONS } from './src/docs-pages.mjs';
import { SHORT_NAME, SITE_NAME, TAGLINE } from './src/brand.mjs';

// Starlight's `autogenerate` only sees src/content/docs, and the docs live in the repository's docs/: the user docs
// sections are listed in docs-pages.mjs, each page labelled with its title. Contributing also lists the developer
// guide (docs/dev/) and SDLC.md.
const sections = USER_SECTIONS.map((s) => ({
  label: s.label,
  items: [...s.pages.map((p) => `user/${s.dir}/${p}.md`), ...(s.more ?? [])].map((f) => ({ slug: docId(f) })),
}));

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
      title: SHORT_NAME,
      description: `${SITE_NAME}: ${TAGLINE}.`,
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
