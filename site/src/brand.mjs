// The product's names as the website spells them (the app's own are in crates/pdt-core/src/brand.rs).

export const SHORT_NAME = 'PDT';
export const NAME = 'Personal Dev Team';
/** In titles, link previews and search results: says what PDT stands for. */
export const SITE_NAME = `${SHORT_NAME} (${NAME})`;
export const TAGLINE = 'a dev team that lives on your machine';

/** A page's `<title>`: the landing page's (no `page`) is the site name and tagline, any other's is `page · site name`. */
export function pageTitle(page) {
  return page ? `${page} · ${SITE_NAME}` : `${SITE_NAME}: ${TAGLINE}`;
}
