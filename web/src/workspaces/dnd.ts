import type { DragEvent } from 'react';
import type { Item } from './model';

/** Drag payloads: a tab (by id) or a tree item (session or run). */
export const TAB_MIME = 'application/x-pdt-tab';
export const ITEM_MIME = 'application/x-pdt-item';

export function dragTab(e: DragEvent, tabId: string) {
  e.dataTransfer.setData(TAB_MIME, tabId);
  e.dataTransfer.effectAllowed = 'move';
}

export function dragItem(e: DragEvent, item: Item, projectId: string) {
  e.dataTransfer.setData(ITEM_MIME, JSON.stringify({ ...item, projectId }));
  e.dataTransfer.effectAllowed = 'move';
}

/** True when the drag carries something a tab strip or workspace accepts (types are readable during dragover). */
export function accepts(e: DragEvent): boolean {
  const types = Array.from(e.dataTransfer?.types ?? []);
  return types.includes(TAB_MIME) || types.includes(ITEM_MIME);
}

export type Dropped = { tabId: string } | { item: Item; projectId: string } | null;

export function dropped(e: DragEvent): Dropped {
  const tabId = e.dataTransfer.getData(TAB_MIME);
  if (tabId) return { tabId };
  try {
    const v = JSON.parse(e.dataTransfer.getData(ITEM_MIME) || 'null');
    if (v && (v.kind === 'session' || v.kind === 'run') && typeof v.id === 'string') {
      return { item: { kind: v.kind, id: v.id }, projectId: String(v.projectId ?? '') };
    }
  } catch {
    // Not ours.
  }
  return null;
}
