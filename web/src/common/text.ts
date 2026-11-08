// Small text helpers, one home each.

/** "1 file", "3 files"; give `many` where adding an s is wrong ("1 entry", "2 entries"). */
export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The first letter upper-cased: "merge conflict" → "Merge conflict". */
export const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** Items in a sentence: "a", "a and b", "a, b and c"; `last` joins the final pair ("or"). */
export const listText = (items: string[], last = 'and') =>
  items.length > 1 ? `${items.slice(0, -1).join(', ')} ${last} ${items[items.length - 1]}` : (items[0] ?? '');

/** `bytes` for people: "512 MB", "1.5 GB" (as the server's Inbox items say it). */
export function formatBytes(bytes: number): string {
  const units = ['bytes', 'KB', 'MB', 'GB', 'TB'];
  let v = bytes;
  let unit = 0;
  while (v >= 1024 && unit < units.length - 1) {
    v /= 1024;
    unit += 1;
  }
  return unit === 0 || v >= 10 ? `${Math.round(v)} ${units[unit]}` : `${v.toFixed(1)} ${units[unit]}`;
}
