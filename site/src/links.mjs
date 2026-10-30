// Where the landing and Features pages link, under the site's base path (`import.meta.env.BASE_URL`).

export const REPO = 'https://github.com/RossTools/PersonalDevTeam';

/** The site's own pages and the project's links. `docs(path)` is a user docs page: `docs('using-pdt/the-inbox')`. */
export function siteLinks(baseUrl) {
  const home = baseUrl.replace(/\/?$/, '/');
  return {
    home,
    features: `${home}features/`,
    docs: `${home}docs/`,
    doc: (path) => `${home}docs/${path}/`,
    changelog: `${REPO}/blob/main/CHANGELOG.md`,
    github: REPO,
    download: `${REPO}/releases/latest`,
  };
}
