// Where the landing, Features and Plugins pages link, under the site's base path (`import.meta.env.BASE_URL`).

export const REPO = 'https://github.com/RossTools/PersonalDevTeam';
/** The port PDT answers on (pdt-core `brand::STABLE_PORT`; a test there checks this line). */
export const STABLE_PORT = 7420;

/** The site's own pages and the project's links. `docs(path)` is a user docs page: `docs('using-pdt/the-inbox')`. */
export function siteLinks(baseUrl) {
  const home = baseUrl.replace(/\/?$/, '/');
  return {
    home,
    features: `${home}features/`,
    plugins: `${home}plugins/`,
    docs: `${home}docs/`,
    privacy: `${home}privacy/`,
    doc: (path) => `${home}docs/${path}/`,
    changelog: `${REPO}/blob/main/CHANGELOG.md`,
    license: `${REPO}/blob/main/LICENSE`,
    github: REPO,
    download: `${REPO}/releases/latest`,
    requestPlugin: `${REPO}/issues/new?template=plugin.yml`,
  };
}
