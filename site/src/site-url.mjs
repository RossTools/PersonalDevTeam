// The URL the site is built for. The Pages workflow passes configure-pages' `origin`, which is `http://` when the
// build runs before Pages enforces HTTPS on a custom domain; Pages serves it over HTTPS, so the install box, canonical
// links, Open Graph URLs and the sitemap always say `https://`. Locally the site is served at http://localhost:4321.

/** The site URL from `SITE_URL` (`env`), with an `http://` origin upgraded to `https://` unless it is localhost. */
export function siteUrl(env = process.env) {
  const url = env.SITE_URL || 'http://localhost:4321';
  const { hostname } = new URL(url);
  const local = hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]';
  return local ? url : url.replace(/^http:\/\//i, 'https://');
}
