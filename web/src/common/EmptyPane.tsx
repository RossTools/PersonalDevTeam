import type { ReactNode } from 'react';
import { Button, EmptyState } from '@ui';

/**
 * The detail pane of a list view (Notes, Documents, Workflows, Personas, Files) before anything is picked, built one
 * way (PDT-678): the view's icon, one line, and an outline New button when there is one to press.
 */
export function EmptyPane({ icon, children, newLabel, onNew }: {
  icon: ReactNode;
  children: ReactNode;
  /** The button's text ("New note"); without `onNew` there is no button. */
  newLabel?: string;
  onNew?: () => void;
}) {
  return (
    <EmptyState icon={icon} className="mt-16 border-0" action={onNew && <Button size="sm" onClick={onNew}>{newLabel}</Button>}>
      {children}
    </EmptyState>
  );
}
