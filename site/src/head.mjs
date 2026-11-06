// The <head> tags every page shares, the landing frame's (layouts/Landing.astro) and the docs' (Starlight's `head`):
// the icons browsers and phones ask for, and the picture a shared link shows. Made by scripts/site-icons.mjs.
import { SITE_NAME, TAGLINE } from './brand.mjs';

/** Link preview image size (what og.png is). */
export const OG_IMAGE = { path: 'og.png', width: 1200, height: 630 };

/** The tags, as Starlight's `head` entries; `site` is the site's URL and `base` its base path. */
export function sharedHead(site, base) {
  const home = base.replace(/\/?$/, '/');
  const image = new URL(`${home}${OG_IMAGE.path}`, site).href;
  const meta = (key, name, content) => ({ tag: 'meta', attrs: { [key]: name, content } });
  return [
    { tag: 'link', attrs: { rel: 'icon', href: `${home}favicon.ico`, sizes: '32x32' } },
    { tag: 'link', attrs: { rel: 'apple-touch-icon', href: `${home}apple-touch-icon.png` } },
    meta('property', 'og:image', image),
    meta('property', 'og:image:width', String(OG_IMAGE.width)),
    meta('property', 'og:image:height', String(OG_IMAGE.height)),
    meta('property', 'og:image:alt', `${SITE_NAME}: ${TAGLINE}`),
    meta('name', 'twitter:image', image),
  ];
}
