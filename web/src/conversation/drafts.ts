/**
 * Unsent message text per session, so a conversation's box keeps what was typed when the user clicks away and back
 * (sessionStorage: this browser tab; the page's own memory when storage is blocked).
 */
const key = (sessionId: string) => `pdt.conversation.draft.${sessionId}`;
const memory = new Map<string, string>();

export function loadDraft(sessionId: string): string {
  try {
    const v = sessionStorage.getItem(key(sessionId));
    if (v !== null) return v;
  } catch {
    // Blocked: the page's memory.
  }
  return memory.get(sessionId) ?? '';
}

export function saveDraft(sessionId: string, text: string) {
  if (text) memory.set(sessionId, text);
  else memory.delete(sessionId);
  try {
    if (text) sessionStorage.setItem(key(sessionId), text);
    else sessionStorage.removeItem(key(sessionId));
  } catch {
    // Blocked: remembered for this page only.
  }
}
