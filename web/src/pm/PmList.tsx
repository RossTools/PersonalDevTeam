import { useState } from 'react';
import { Plus } from 'lucide-react';
import { IconButton, List, ListHeading, ListRow, relativeTime, Skeleton, Spinner, useMenuAt } from '@ui';
import { InlineRename } from '../common/InlineRename';
import { RowField } from '../common/RowField';
import { earlier, firstLine, isUnread, openConversations, REASON_LABEL, resetting, shownConversation, usePm, usePmActions, useShownRef, useViaLabel, type PmSelection } from './usePm';
import { LoadError } from '../common/QueryStatus';

/** Ended conversations the list shows; the rest are under "All ended conversations". */
const RECENT_ENDED = 5;

/**
 * The PM view's list column (PDT-149): the open conversations (the most recently active first, an unread one in bold
 * with a dot and a working one with a spinner, PDT-205; each renamed, reset or closed from its menu), New conversation, then
 * the recently ended ones (their journal's first line). On a phone it is the PM's first screen. The notes have a view of their own (PDT-143).
 */
export function PmList({ selection, onSelect, onNew }: {
  selection: PmSelection;
  onSelect: (s: PmSelection) => void;
  onNew: () => void;
}) {
  const pm = usePm();
  const viaLabel = useViaLabel();
  const sessions = pm.data?.sessions ?? [];
  const open = openConversations(sessions);
  const ended = earlier(sessions);
  const shown = shownConversation(sessions, pm.data?.current?.id, selection)?.session.id ?? null;
  const { openMenu, menu } = useMenuAt();
  const [renaming, setRenaming] = useState<string | null>(null);
  const onShow = useShownRef(shown);
  const { rename, close, reset } = usePmActions((id) => {
    if (id === shown) onSelect('current');
  }, (id, fresh) => {
    // The fresh conversation takes the old one's place on screen too, if it is still there (PDT-308).
    if (id === onShow.current) onSelect(fresh);
  });
  return (
    <div className="flex flex-col gap-0.5 py-1">
      <ListHeading
        trailing={
          <IconButton label="New conversation" size="sm" noTooltip className="size-5" onClick={onNew}>
            <Plus aria-hidden />
          </IconButton>
        }
      >
        Project manager
      </ListHeading>
      {pm.isPending && <Skeleton lines={3} label="Loading the PM" />}
      <LoadError what="The PM" error={pm.error} />
      <List aria-label="PM conversations" onAction={(id) => onSelect(String(id))}>
        {open.map((info) => {
          const unread = isUnread(info, shown);
          return <ListRow
            key={info.session.id}
            id={info.session.id}
            textValue={info.title}
            title={renaming === info.session.id ? (
              <RowField>
                <InlineRename
                  label="Conversation name"
                  initial={info.title}
                  onCommit={(t) => { setRenaming(null); rename(info.session.id, t); }}
                  onCancel={() => setRenaming(null)}
                />
              </RowField>
            ) : info.title}
            current={shown === info.session.id}
            bold={unread}
            trailing={unread ? <span role="img" aria-label="New reply" className="size-2 rounded-full bg-primary" />
              : info.session.activity === 'working' ? <Spinner decorative className="size-3 text-muted-foreground" /> : undefined}
            secondary={[resetting(info) ? 'Resetting…' : info.rotation === 'journal' ? 'Writing its journal…'
              : info.session.activity === 'working' ? 'Working…' : relativeTime(info.lastActivity), viaLabel(info)]
              .filter(Boolean).join(' · ')}
            onMenu={(at) => openMenu(at, `Conversation ${info.title}`, [
              { label: 'Rename', onSelect: () => setRenaming(info.session.id) },
              { label: 'Reset…', onSelect: () => reset(info), disabled: info.rotation !== 'none' },
              { label: 'Close…', onSelect: () => close(info), disabled: info.rotation !== 'none' },
            ])}
          />;
        })}
        {(open.length === 0 || selection === 'new') && (
          <ListRow id="new" title="New conversation" current={selection === 'new' || (!shown && selection === 'current')}
            secondary="Ask something to start one" />
        )}
      </List>
      {ended.length > 0 && (
        <>
          <ListHeading as="h3">Ended</ListHeading>
          <List aria-label="Ended PM conversations" onAction={(id) => onSelect(String(id))}>
            {ended.slice(0, RECENT_ENDED).map((s) => (
              <ListRow
                key={s.session.id}
                id={s.session.id}
                title={s.title}
                current={shown === s.session.id}
                secondary={firstLine(s.journal) || (s.reason ? `Ended: ${REASON_LABEL[s.reason]}` : 'Ended')}
              />
            ))}
            {ended.length > RECENT_ENDED && (
              <ListRow id="earlier" title="All ended conversations" current={selection === 'earlier'} secondary={`${ended.length} in all`} />
            )}
          </List>
        </>
      )}
      {menu}
    </div>
  );
}
