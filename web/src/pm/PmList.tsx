import { Plus } from 'lucide-react';
import { Alert, IconButton, List, ListRow, Skeleton, relativeTime } from '@ui';
import { earlier, firstLine, REASON_LABEL, usePm, type PmSelection } from './usePm';

const heading = 'flex items-center px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';

/**
 * The PM view's list column (PDT-81): the current session and the sessions that ended (when, why, and their journal's
 * first line). The notes have a view of their own (PDT-143).
 */
export function PmList({ selection, onSelect, onNew }: {
  selection: PmSelection;
  onSelect: (s: PmSelection) => void;
  onNew: () => void;
}) {
  const pm = usePm();
  const current = pm.data?.current ?? null;
  const ended = earlier(pm.data?.sessions ?? []);
  return (
    <div className="flex flex-col gap-0.5 py-1">
      <h2 className={heading}>
        <span className="flex-1">Project manager</span>
        <IconButton label="New PM session" size="sm" noTooltip className="size-5" onClick={onNew}>
          <Plus aria-hidden />
        </IconButton>
      </h2>
      {pm.isPending && <Skeleton lines={3} label="Loading the PM" />}
      {pm.isError && <Alert tone="error">The PM could not be loaded: {pm.error.message}</Alert>}
      <List aria-label="Project manager" onAction={(id) => onSelect(String(id))}>
        <ListRow
          id="current"
          title="Current session"
          current={selection === 'current'}
          secondary={current ? `Started ${relativeTime(current.createdAt)}` : 'Ask something to start one'}
        />
      </List>
      {ended.length > 0 && (
        <>
          <h3 className={heading}>Earlier sessions</h3>
          <List aria-label="Earlier PM sessions" onAction={(id) => onSelect(String(id))}>
            {ended.map((s) => (
              <ListRow
                key={s.session.id}
                id={s.session.id}
                title={new Date(s.session.createdAt).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })}
                current={selection === s.session.id}
                secondary={firstLine(s.journal) || (s.reason ? `Ended: ${REASON_LABEL[s.reason]}` : 'Ended')}
              />
            ))}
          </List>
        </>
      )}
    </div>
  );
}
