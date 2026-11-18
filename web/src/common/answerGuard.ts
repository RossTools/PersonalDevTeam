// The guard on answers (PDT-623): a fast double-click answers one thing, and a click that lands on whatever slid into
// place after an answer does nothing. While an answer is going out, and for a moment after, no other answer starts.
import { useSyncExternalStore } from 'react';

/** How long after an answer the next one waits (ms). Tests set it to 0 (test-setup.ts). */
export const answerTiming = { cooldownMs: 500 };

let active = 0;
let until = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();
const changed = () => listeners.forEach((l) => l());

const held = () => active > 0 || Date.now() < until;

/** Starts an answer: false (do nothing) while another is going out or just went. Pair each true with `endAnswer`. */
export function beginAnswer(): boolean {
  if (held()) return false;
  active += 1;
  changed();
  return true;
}

/** An answer went out (or failed): the next waits for the cool-down. */
export function endAnswer() {
  active = Math.max(0, active - 1);
  until = Date.now() + answerTiming.cooldownMs;
  clearTimeout(timer);
  timer = setTimeout(changed, answerTiming.cooldownMs);
  changed();
}

/** Runs `f` as one guarded answer: undefined, without running it, while answers are held. */
export async function guardedAnswer<T>(f: () => Promise<T>): Promise<T | undefined> {
  if (!beginAnswer()) return undefined;
  try {
    return await f();
  } finally {
    endAnswer();
  }
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/** Whether answers are held right now: actions show as off, then fade back in. */
export function useAnswersHeld(): boolean {
  return useSyncExternalStore(subscribe, held);
}

/** For tests: nothing held. */
export function resetAnswerGuard() {
  active = 0;
  until = 0;
  clearTimeout(timer);
}
