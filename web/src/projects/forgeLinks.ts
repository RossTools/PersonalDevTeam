import type { ForgeLinks } from '../api/gen/ForgeLinks';

/** Links into a project's repository on its forge, from the pages the forge plugin names. Branch names keep their slashes; other characters are encoded. */
const branch = (b: string) => b.split('/').map(encodeURIComponent).join('/');

// A function replacer, so a `$` in a branch name is not a replacement pattern.
const fill = (template: string, values: Record<string, string>) =>
  Object.entries(values).reduce((url, [k, v]) => url.replace(`{${k}}`, () => v), template);

export const repoUrl = (forge: ForgeLinks) => forge.repoUrl;
/** The branch's page; null when the forge has none. */
export const branchUrl = (forge: ForgeLinks, b: string) => (forge.branchUrl ? fill(forge.branchUrl, { branch: branch(b) }) : null);
/** Opens the change request for `b` against `base`, or the form to make one; null when the forge has no such page. */
export const compareUrl = (forge: ForgeLinks, base: string, b: string) =>
  forge.compareUrl ? fill(forge.compareUrl, { base: branch(base), branch: branch(b) }) : null;
