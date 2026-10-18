import { useEffect, useState } from 'react';

/** Whether a CSS media query matches, kept current. False where `matchMedia` is missing. */
export function useMediaQuery(query: string): boolean {
  const get = () => typeof window !== 'undefined' && !!window.matchMedia?.(query)?.matches;
  const [matches, setMatches] = useState(get);
  useEffect(() => {
    const mq = window.matchMedia?.(query);
    if (!mq) return;
    const on = () => setMatches(mq.matches);
    on();
    mq.addEventListener?.('change', on);
    return () => mq.removeEventListener?.('change', on);
  }, [query]);
  return matches;
}

/** Below Tailwind's `md` breakpoint: the phone layout (bottom rail, sidebar in a sheet). */
export const PHONE_QUERY = '(max-width: 767.98px)';
