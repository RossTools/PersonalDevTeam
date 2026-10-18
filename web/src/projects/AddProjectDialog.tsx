import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import type { Project } from '../api/gen/Project';
import { AddProjectForm } from './AddProjectForm';

/** The add-project form in a modal (from the sidebar's "Add project" link). Escape or the backdrop closes it. */
export function AddProjectDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (p: Project) => void }) {
  const dialog = useRef<HTMLDivElement>(null);
  // Focus the first field once, on open (never on later renders, which would steal focus while typing).
  useEffect(() => dialog.current?.querySelector<HTMLElement>('input')?.focus(), []);
  return createPortal(
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-black/30 p-4 pt-[10vh]" onClick={onClose}>
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-label="Add project"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}
        className="w-full max-w-md rounded-pdt border border-border bg-surface shadow-lg"
      >
        <div className="flex items-center border-b border-border px-3 py-2">
          <h2 className="flex-1 text-sm font-medium">Add project</h2>
          <button type="button" aria-label="Close" onClick={onClose} className="rounded-pdt p-1 text-muted-foreground hover:text-fg hover:bg-surface-2">
            <X size={14} aria-hidden />
          </button>
        </div>
        <AddProjectForm onAdded={(p) => { onAdded(p); onClose(); }} />
      </div>
    </div>,
    document.body,
  );
}
