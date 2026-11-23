import type { ReactNode } from 'react';
import { Plus } from 'lucide-react';
import { Button } from '@ui';
import { PHONE_QUERY, useMediaQuery } from './useMediaQuery';

/**
 * The top of a view's list column (Notes, Documents, Workflows, Personas, Files), built one way (PDT-678): the title,
 * then any `extra` icon buttons, then `+ New` (named `newLabel` for screen readers). On a phone the top bar already
 * shows the title, so the row keeps only its buttons, and is left out when it has none.
 */
export function ListColumnHeader({ title, newLabel, onNew, newDisabled, extra }: {
  title: string;
  /** What New makes, in full ("New note"); without `onNew` there is no New. */
  newLabel?: string;
  onNew?: () => void;
  newDisabled?: boolean;
  /** Icon buttons or a `⋯` menu before New. */
  extra?: ReactNode;
}) {
  const phone = useMediaQuery(PHONE_QUERY);
  const actions = !!onNew || !!extra;
  if (phone && !actions) return null;
  return (
    <div className="flex min-h-control-md items-center gap-1 pt-2.5 pr-1.5 pb-2 pl-3">
      {phone ? <span className="flex-1" /> : <h2 className="flex-1 truncate text-sm font-semibold tracking-tight">{title}</h2>}
      {extra}
      {onNew && (
        <Button size="sm" aria-label={newLabel} disabled={newDisabled} onClick={onNew}>
          <Plus aria-hidden /> New
        </Button>
      )}
    </div>
  );
}
