// The two-press confirm (PDT-623): an action that acts outside PDT or can't be taken back arms on its first press and
// acts on a second, never the second click of a double-click. It disarms after a few seconds, and says so for a few
// more (PDT-667: what a screen reader hears).
import { useEffect, useState } from 'react';

/** How soon a confirming press counts after the first, and how long it stays armed (ms). Tests set `confirmAfterMs` 0. */
export const twoPressTiming = { confirmAfterMs: 500, disarmMs: 4000 };

export interface TwoPress<K extends string> {
  /** The action waiting for its second press. */
  armed: K | null;
  /** An action disarmed without its second press, a moment ago (say "Not confirmed"). */
  lapsed: boolean;
  /** A press on `key`: the first arms it, a second (not inside a double-click) disarms it and runs `act`. */
  press: (key: K, act: () => void) => void;
}

/** Arm-then-confirm for one or more actions, keyed: arming one disarms the others. */
export function useTwoPress<K extends string = string>(): TwoPress<K> {
  const [armed, setArmed] = useState<{ key: K; at: number } | null>(null);
  const [lapsed, setLapsed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    setLapsed(false);
    const t = setTimeout(() => { setArmed(null); setLapsed(true); }, twoPressTiming.disarmMs);
    return () => clearTimeout(t);
  }, [armed]);
  useEffect(() => {
    if (!lapsed) return;
    const t = setTimeout(() => setLapsed(false), twoPressTiming.disarmMs);
    return () => clearTimeout(t);
  }, [lapsed]);
  const press = (key: K, act: () => void) => {
    if (armed?.key !== key) return setArmed({ key, at: Date.now() });
    if (Date.now() - armed.at < twoPressTiming.confirmAfterMs) return;
    setArmed(null);
    act();
  };
  return { armed: armed?.key ?? null, lapsed, press };
}
