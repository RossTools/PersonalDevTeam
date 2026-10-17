/** Links into a GitHub repository (`owner/repo`). Branch names keep their slashes; other characters are encoded. */
const branch = (b: string) => b.split('/').map(encodeURIComponent).join('/');

export const repoUrl = (repo: string) => `https://github.com/${repo}`;
export const branchUrl = (repo: string, b: string) => `${repoUrl(repo)}/tree/${branch(b)}`;
/** Opens the pull request for `b` against `base`, or the form to make one. */
export const compareUrl = (repo: string, base: string, b: string) => `${repoUrl(repo)}/compare/${branch(base)}...${branch(b)}?expand=1`;
