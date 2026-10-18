import type { DragEvent } from 'react';
import { dragData, parseDragData } from '@ui';
import type { Item } from './model';

/**
 * Drag payloads are text/plain `pdt:<kind>:<id>` (web/ui README: Android Chrome drops custom types): a tab
 * (`pdt:tab:<tab id>`), a session (`pdt:session:<id>`, from the sidebar's agent list) or a run (`pdt:run:<id>`), and a
 * workspace (`pdt:workspace:<id>`, reordering the sidebar list).
 */
export const DROP_KINDS = ['tab', 'session', 'run'];

/** Starts a native drag of a tab (the tab strip isn't a List). */
export function dragTab(e: DragEvent, tabId: string) {
  e.dataTransfer.setData('text/plain', dragData('tab', tabId));
  e.dataTransfer.effectAllowed = 'move';
}

/** True when the drag may carry something a tab strip accepts (only the types are readable during dragover). */
export function accepts(e: DragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes('text/plain');
}

export type Dropped = { tabId: string } | { item: Item } | null;

/** What a `pdt:<kind>:<id>` payload (a native drop, or a List's `ListDropEvent`) means to a tab strip or workspace. */
export function droppedOf(p: { kind: string; id: string } | null): Dropped {
  if (!p) return null;
  if (p.kind === 'tab') return { tabId: p.id };
  if (p.kind === 'session' || p.kind === 'run') return { item: { kind: p.kind, id: p.id } };
  return null;
}

/** A native drop on the tab strip. */
export function dropped(e: DragEvent): Dropped {
  return droppedOf(parseDragData(e.dataTransfer.getData('text/plain') ?? ''));
}
