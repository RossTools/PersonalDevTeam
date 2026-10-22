import type { SelectOption } from '@ui';
import type { Branch } from '../api/gen/Branch';

/** Branches offered first, in this order, when the repository has them (then `also`). */
const FIRST = ['main', 'master'];

/**
 * A branch picker's options: `main` (then `master`, then `also`, e.g. the project's base branch) first, the rest in
 * git's order. Branches only on origin say so. `also` is added when the list lacks it (a base branch since deleted).
 */
export function branchOptions(branches: Branch[], also?: string): SelectOption[] {
  const first = [...FIRST, ...(also && !FIRST.includes(also) ? [also] : [])];
  const rank = (name: string) => {
    const i = first.indexOf(name);
    return i < 0 ? first.length : i;
  };
  const options = [...branches]
    .sort((a, b) => rank(a.name) - rank(b.name))
    .map((b) => ({ value: b.name, label: b.local ? b.name : `${b.name} (origin)` }));
  if (also && !options.some((o) => o.value === also)) options.unshift({ value: also, label: also });
  return options;
}
