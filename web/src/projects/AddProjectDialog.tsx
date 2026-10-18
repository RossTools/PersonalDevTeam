import { Dialog } from '@ui';
import type { Project } from '../api/gen/Project';
import { AddProjectForm } from './AddProjectForm';

/** The add-project form in a modal (from the Sessions sidebar's "Add project" and the "+" next to PROJECTS in Settings). Escape, Close or the backdrop closes it. */
export function AddProjectDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (p: Project) => void }) {
  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }} title="Add project">
      <AddProjectForm autoFocus onAdded={(p) => { onAdded(p); onClose(); }} />
    </Dialog>
  );
}
