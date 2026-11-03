// Small text helpers, one home each.

/** "1 file", "3 files"; give `many` where adding an s is wrong ("1 entry", "2 entries"). */
export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The first letter upper-cased: "merge conflict" → "Merge conflict". */
export const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

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
