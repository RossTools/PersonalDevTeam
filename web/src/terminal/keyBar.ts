// The phone's terminal key bar (PDT-115): keys a soft keyboard can't send, a sticky Ctrl and PDT's Ctrl+] prefix.

/** A key bar key that is armed and changes what the next key does. */
export type Pending = 'ctrl' | 'prefix' | null;

/** The key bar's keys that send bytes; Ctrl, ⌃] and ⌨ are handled by the bar itself. */
export const SEND_KEYS: { id: string; label: string; aria: string; seq: string }[] = [
  { id: 'esc', label: 'Esc', aria: 'Escape', seq: '\x1b' },
  { id: 'tab', label: 'Tab', aria: 'Tab', seq: '\t' },
  { id: 'ctrl-c', label: '^C', aria: 'Control C', seq: '\x03' },
  { id: 'up', label: '↑', aria: 'Up', seq: '\x1b[A' },
  { id: 'down', label: '↓', aria: 'Down', seq: '\x1b[B' },
  { id: 'left', label: '←', aria: 'Left', seq: '\x1b[D' },
  { id: 'right', label: '→', aria: 'Right', seq: '\x1b[C' },
];

/** Ctrl+]: what the terminal sends for it (PDT's prefix pressed twice). */
const PREFIX_BYTE = '\x1d';

/**
 * The byte Ctrl plus one character sends, as a terminal does (Ctrl+C is 0x03, Ctrl+[ is ESC, Ctrl+? is DEL), and
 * Ctrl plus an arrow (or Home/End) as xterm's modified cursor keys (`ESC [ 1 ; 5 A`). Null for anything else.
 */
export function withCtrl(data: string): string | null {
  if (data.length === 1) {
    const c = (/[a-z]/.test(data) ? data.toUpperCase() : data).charCodeAt(0);
    if (c >= 0x40 && c <= 0x5f) return String.fromCharCode(c - 0x40); // @ A–Z [ \ ] ^ _
    if (data === ' ') return '\x00';
    if (data === '?') return '\x7f';
    if (data === '/') return '\x1f';
    return null;
  }
  const arrow = /^\x1b[[O]([ABCDHF])$/.exec(data);
  return arrow ? `\x1b[1;5${arrow[1]}` : null;
}

/**
 * What a terminal input (typed on the soft keyboard, or a key bar key) does while a key is armed: the bytes to send,
 * or the key to run as a PDT shortcut (after ⌃]). Input the armed key doesn't apply to (a mouse report, a paste)
 * goes through unchanged and leaves it armed.
 */
export function applyPending(pending: Pending, data: string): { send?: string; prefixKey?: string; pending: Pending } {
  if (pending === 'ctrl') {
    const ctrl = withCtrl(data);
    return ctrl === null ? { send: data, pending } : { send: ctrl, pending: null };
  }
  if (pending === 'prefix') {
    // One typed character (not an escape sequence or a paste) is the shortcut's key.
    if ([...data].length === 1 && data >= ' ') return { prefixKey: data, pending: null };
    return { send: data, pending };
  }
  return { send: data, pending };
}

/**
 * A key bar key (Esc, an arrow…) while a key is armed. Ctrl applies when the key has a Ctrl form and lets go either
 * way; after ⌃] a key bar key is no shortcut, so it is dropped and the prefix lets go (as a keyboard's Esc would).
 */
export function pressBarKey(pending: Pending, seq: string): { send?: string; pending: Pending } {
  if (pending === 'ctrl') return { send: withCtrl(seq) ?? seq, pending: null };
  if (pending === 'prefix') return { pending: null };
  return { send: seq, pending };
}

/** Tapping ⌃]: arms PDT's prefix; tapped again (as on a keyboard), it sends Ctrl+] to the program instead. */
export function tapPrefix(pending: Pending): { send?: string; pending: Pending } {
  return pending === 'prefix' ? { send: PREFIX_BYTE, pending: null } : { pending: 'prefix' };
}

/** Tapping Ctrl arms it for the next key; tapped again, it lets go. */
export function tapCtrl(pending: Pending): Pending {
  return pending === 'ctrl' ? null : 'ctrl';
}

/** The terminal font size: kept per browser, so a size picked on the phone sticks. */
export const FONT_SIZE = { default: 13, min: 9, max: 24 };

export function clampFontSize(n: number): number {
  return Number.isFinite(n) ? Math.min(FONT_SIZE.max, Math.max(FONT_SIZE.min, Math.round(n))) : FONT_SIZE.default;
}

/**
 * The word (letters, digits and path characters) at column `col` of a line, as [start, end) columns; a single cell
 * when there is no word there.
 */
export function wordAt(line: string, col: number): [number, number] {
  const isWord = (ch: string | undefined) => !!ch && /[\w\-./~:@%+=#?&]/.test(ch);
  if (!isWord(line[col])) return [col, col + 1];
  let start = col;
  let end = col + 1;
  while (start > 0 && isWord(line[start - 1])) start--;
  while (end < line.length && isWord(line[end])) end++;
  return [start, end];
}
