// The site's /.well-known/security.txt (RFC 9116): where to report a vulnerability. It is written at build time, so
// `Expires` moves on with every deploy; pages.yml also redeploys monthly, so it never runs out.
import { REPO } from './links.mjs';

/** How long a deploy's file stays valid. RFC 9116 asks for less than a year. */
export const VALID_DAYS = 180;

/** The file for a site at `canonical` (its full URL), built at `now`. */
export function securityTxt(canonical, now = new Date()) {
  const expires = new Date(now.getTime() + VALID_DAYS * 24 * 60 * 60 * 1000);
  expires.setUTCMilliseconds(0);
  return [
    'Contact: mailto:security@pdt.dev',
    `Contact: ${REPO}/security/advisories/new`,
    `Expires: ${expires.toISOString().replace('.000Z', 'Z')}`,
    'Preferred-Languages: en',
    `Policy: ${REPO}/blob/main/SECURITY.md`,
    `Canonical: ${canonical}`,
    '',
  ].join('\n');
}
