import { Dialog } from '@ui';
import type { Project } from '../api/gen/Project';
import { AddProjectForm, type AddProjectMode } from './AddProjectForm';

/**
 * The add-project form in a modal (from the Sessions sidebar's "Add project", the "+" next to PROJECTS in Settings and
 * Get started). Escape, Close or the backdrop closes it. `mode`: the way of adding it opens on.
 */
export function AddProjectDialog({ onClose, onAdded, mode }: { onClose: () => void; onAdded: (p: Project) => void; mode?: AddProjectMode }) {
  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }} title="Add project">
      <AddProjectForm autoFocus initialMode={mode} onAdded={(p) => { onAdded(p); onClose(); }} />
    </Dialog>
  );
}
