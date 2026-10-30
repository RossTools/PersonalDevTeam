/**
 * `text` with `insert` put at the text box's caret (replacing a selection), on a line of its own; at the end when there
 * is no box.
 */
export function insertAtCaret(text: string, box: HTMLTextAreaElement | null, insert: string): string {
  const start = box?.selectionStart ?? text.length;
  const end = box?.selectionEnd ?? text.length;
  const before = text.slice(0, start);
  const after = text.slice(end);
  const lead = before && !before.endsWith('\n') ? '\n' : '';
  const trail = after && !after.startsWith('\n') ? '\n' : '';
  return `${before}${lead}${insert}${trail}${after}`;
}
