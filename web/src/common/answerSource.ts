// How the user gave an answer (PDT-623): the last press before it was a pointer's (click or tap) or a key's, on a
// phone or not. The Inbox's Recently answered says so.
import type { AnswerSource } from '../api/gen/AnswerSource';

let lastVia: AnswerSource['via'] = 'click';

if (typeof window !== 'undefined') {
  window.addEventListener('pointerdown', () => { lastVia = 'click'; }, { capture: true });
  window.addEventListener('keydown', () => { lastVia = 'keyboard'; }, { capture: true });
}

/** How the answer being given now was given. `phone`: the phone's layout, else a coarse pointer says so. */
export function answerSource(phone?: boolean): AnswerSource {
  const coarse = typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;
  return { via: lastVia, device: (phone ?? coarse) ? 'phone' : 'desktop' };
}
