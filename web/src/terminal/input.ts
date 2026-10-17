/**
 * Sent for Shift+Enter. xterm sends `\r` for Enter with or without Shift, so agents can't tell them apart; Meta+Enter
 * (ESC, CR) inserts a line break in Claude Code and Codex, and in zsh.
 */
export const NEWLINE = '\x1b\r';

export function isShiftEnter(e: KeyboardEvent): boolean {
  return e.key === 'Enter' && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey;
}

/**
 * The bytes that type `text` into the program as one paste and then press Enter. Line breaks become `\r`, as a
 * terminal sends them. When the program enabled bracketed paste the text is wrapped in its markers, so agents keep
 * the line breaks in the prompt instead of submitting at each one; escape characters are dropped so the text can't
 * close the paste early.
 */
export function promptSequence(text: string, bracketed: boolean): string {
  const body = text.replace(/\x1b/g, '').replace(/\r?\n/g, '\r');
  return (bracketed ? `\x1b[200~${body}\x1b[201~` : body) + '\r';
}

/** Web links open on Cmd/Ctrl+click (a plain click belongs to the program's own mouse handling), or a tap on touch. */
export function shouldOpenLink(e: MouseEvent, uri: string, touch: boolean): boolean {
  let url: URL;
  try {
    url = new URL(uri);
  } catch {
    return false;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
  return touch || e.metaKey || e.ctrlKey;
}
