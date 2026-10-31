// A local document's live changes (PDT-315), from pdtd's event stream for it: a sheet's diffs and reloads, a doc's
// saves, and its comments and charts. The browser's EventSource reconnects by itself.
import { useEffect, useRef } from 'react';
import { api } from '../api/client';

export type DocEvent =
  | { type: 'diffs'; seq: number; from: string; data: string }
  | { type: 'reload'; seq: number }
  | { type: 'changed'; version: string }
  | { type: 'comments' }
  | { type: 'charts' };

const NAMES = ['diffs', 'reload', 'changed', 'comments', 'charts'] as const;

/** This tab's id as a view of documents, so pdtd doesn't send a sheet's edits back to the view that made them. */
export const VIEW_ID = `v${Math.random().toString(36).slice(2, 10)}`;

/** Calls `onEvent` for each change while `enabled`. `onOpen` runs on every (re)connect, to catch up on missed ones. */
export function useDocumentEvents(id: string, enabled: boolean, onEvent: (e: DocEvent) => void, onOpen?: () => void) {
  const handler = useRef(onEvent);
  handler.current = onEvent;
  const opened = useRef(onOpen);
  opened.current = onOpen;
  useEffect(() => {
    if (!enabled || typeof EventSource === 'undefined') return;
    const source = new EventSource(api.documentEventsUrl(id, VIEW_ID));
    let first = true;
    source.onopen = () => {
      if (!first) opened.current?.();
      first = false;
    };
    const listen = (e: MessageEvent<string>) => {
      try {
        handler.current(JSON.parse(e.data) as DocEvent);
      } catch {
        // A malformed event: ignore it.
      }
    };
    for (const n of NAMES) source.addEventListener(n, listen as EventListener);
    return () => source.close();
  }, [id, enabled]);
}

/** Base64 (as the stream sends diffs) to bytes. */
export function fromBase64(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
