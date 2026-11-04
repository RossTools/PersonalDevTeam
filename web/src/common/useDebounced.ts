import { useEffect, useState } from 'react';

/** Waits until typing pauses before asking the API: `value`, once it has stayed the same for `ms`. */
export function useDebounced<T>(value: T, ms = 200): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}
