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
    releases: `${REPO}/releases/latest`,
    requestPlugin: `${REPO}/issues/new?template=plugin.yml`,
  };
}

/** The newest stable release's file for `os` (`mac`, `windows` or `linux`), straight from GitHub: release assets carry
 * no version in their names (the bundle action names them), so releases/latest/download/ always has them. Linux gets
 * the x86_64 AppImage, the desktop one; the releases page has the rest. */
export function downloadUrl(os) {
  const asset = { mac: 'Personal.Dev.Team.app.tar.gz', windows: 'pdt-windows-x64-setup.exe', linux: 'pdt-linux-x86_64.AppImage' }[os];
  return `${REPO}/releases/latest/download/${asset}`;
}
