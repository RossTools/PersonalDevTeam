/** GitHub's bug form, filled in (PDT-559): where Report a bug sends the user when PDT has no GitHub login or token
 *  to file the issue with. The field ids are `.github/ISSUE_TEMPLATE/bug.yml`'s. */

/** The most of each text field the link carries: GitHub refuses links much over 8 KB. */
const MAX_FIELD = 1500;

const cut = (s: string) => (s.length > MAX_FIELD ? `${s.slice(0, MAX_FIELD - 1)}…` : s);

export interface BugForm {
  repo: string;
  title: string;
  whatHappened: string;
  expected: string;
  steps: string;
  version: string;
  /** Left out when the diagnostics are not shared, as a filed report does. */
  os?: string;
}

/** The link to `repo`'s bug form with the report in it. */
export function bugFormUrl(f: BugForm): string {
  const q = new URLSearchParams({ template: 'bug.yml', title: f.title.trim() });
  const fields: [string, string | undefined][] = [
    ['what-happened', f.whatHappened], ['expected', f.expected], ['steps', f.steps], ['version', f.version], ['os', f.os],
  ];
  for (const [id, v] of fields) if (v?.trim()) q.set(id, cut(v.trim()));
  return `https://github.com/${f.repo}/issues/new?${q.toString()}`;
}
