/**
 * Whether keyboard focus has nowhere to be: the focused control went away (a card answered, a button that hid itself)
 * and the browser dropped focus on the body. Callers then move it somewhere that still means something, but never take
 * it from where the user put it.
 */
export function focusLost(): boolean {
  const el = document.activeElement;
  return el === null || el === document.body;
}
