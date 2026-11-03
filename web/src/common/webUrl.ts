/** Whether the browser may go to an address: only http(s), never `javascript:`, `data:` or `file:` (a link can come
 * from outside PDT, a workflow trigger's payload or a service). */
export function isWebUrl(url: string): boolean {
  try {
    return ['https:', 'http:'].includes(new URL(url).protocol);
  } catch {
    return false;
  }
}
