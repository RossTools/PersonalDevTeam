// The site's own links (the landing page's and the Plugins pages' nav and footer), under the deployed base path.

export const REPO = 'https://github.com/RossTools/PersonalDevTeam';

/** The links, for `base` (Astro's `import.meta.env.BASE_URL`). */
export function siteLinks(base) {
  const root = base.replace(/\/?$/, '/');
  return {
    root,
    docs: `${root}docs/`,
    plugins: `${root}plugins/`,
    changelog: `${REPO}/blob/main/CHANGELOG.md`,
    github: REPO,
    download: `${REPO}/releases/latest`,
    requestPlugin: `${REPO}/issues/new?template=plugin.yml`,
  };
}
