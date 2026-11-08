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

/** An image attached to an unsent message, uploaded already (`id`), with its preview's object URL. */
export interface ImageDraft {
  key: string;
  name: string;
  url: string;
  id: string;
}

/**
 * Uploaded images of unsent messages per session, kept with the text for as long as the page is open (an object URL
 * does not outlive the page, so these are not stored).
 */
const images = new Map<string, ImageDraft[]>();

export function loadImageDrafts(sessionId: string): ImageDraft[] {
  return images.get(sessionId) ?? [];
}

export function saveImageDrafts(sessionId: string, list: ImageDraft[]) {
  if (list.length) images.set(sessionId, list);
  else images.delete(sessionId);
}

/** Forgets every draft the page holds (tests; sessionStorage is cleared with the rest of storage). */
export function resetDrafts() {
  memory.clear();
  images.clear();
}
