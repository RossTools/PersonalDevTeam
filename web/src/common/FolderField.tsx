import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query';
import { FolderOpen } from 'lucide-react';
import { useCallback, useRef, useState } from 'react';
import { Button, Dialog, DialogActions, FolderPicker, Input, type FolderPickerSelection } from '@ui';
import { api } from '../api/client';
import type { FolderListing } from '../api/gen/FolderListing';
import { PHONE_QUERY, useMediaQuery } from './useMediaQuery';
import { hostKeys } from '../host/queries';

export interface FolderFieldProps {
  value: string;
  onChange: (value: string) => void;
  /** Where Browse opens when the field is empty (absolute or `~/…`; empty: the home folder). */
  start?: string;
  /** A folder was chosen in the picker (e.g. to suggest a project name from it). */
  onPick?: (folder: FolderPickerSelection) => void;
  /** The value is final: the field lost focus, Enter was pressed, or a folder was picked (settings save then). */
  onCommit?: (value: string) => void;
  placeholder?: string;
  required?: boolean;
  autoFocus?: boolean;
  'aria-label'?: string;
  /** The picker's title. */
  title?: string;
}

/**
 * A path field with Browse…: type a folder on the host, or pick one in a dialog that lists the host's folders
 * (`/api/host/folders`). The picked path is written as the host writes it, with the home folder as `~`.
 */
export function FolderField({ value, onChange, start = '', onPick, onCommit, placeholder, required, autoFocus, title = 'Choose a folder', ...aria }: FolderFieldProps) {
  const [open, setOpen] = useState(false);
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <Input
        mono
        className="min-w-0 flex-1"
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        required={required}
        autoFocus={autoFocus}
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={(e) => {
          // Browse… takes focus from the field; its pick commits instead.
          if (onCommit && !(e.relatedTarget instanceof HTMLElement && e.relatedTarget.dataset.browse !== undefined)) onCommit(value);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && onCommit) {
            e.preventDefault();
            onCommit(value);
          }
        }}
        {...aria}
      />
      <Button data-browse="" variant="secondary" aria-label={aria['aria-label'] ? `Browse for ${aria['aria-label']}` : 'Browse folders'} onClick={() => setOpen(true)}>
        <FolderOpen /> Browse…
      </Button>
      {open && (
        <FolderDialog
          title={title}
          initial={value.trim() || start}
          onClose={() => setOpen(false)}
          onChoose={(f) => {
            onChange(f.display);
            onPick?.(f);
            onCommit?.(f.display);
            setOpen(false);
          }}
        />
      )}
    </div>
  );
}

function FolderDialog({
  title,
  initial,
  onClose,
  onChoose,
}: {
  title: string;
  initial: string;
  onClose: () => void;
  onChoose: (f: FolderPickerSelection) => void;
}) {
  const qc = useQueryClient();
  const phone = useMediaQuery(PHONE_QUERY);
  // The first listing walks up to the nearest folder that exists (the field may hold a folder not made yet).
  const [target, setTarget] = useState({ path: initial, nearest: true });
  const [hidden, setHidden] = useState(false);
  const [selection, setSelection] = useState<FolderPickerSelection | null>(null);
  const q = useQuery({
    queryKey: hostKeys.folder(target.path, target.nearest, hidden),
    queryFn: () => api.listFolders({ path: target.path, hidden, nearest: target.nearest }),
    placeholderData: keepPreviousData,
    retry: false,
  });
  // An error leaves the last folder that loaded on screen.
  const last = useRef<FolderListing | undefined>(undefined);
  if (q.data && !q.isPlaceholderData) last.current = q.data;
  const listing = q.isError ? last.current : q.data;
  const onBrowse = useCallback((path: string) => setTarget({ path, nearest: false }), []);
  const create = async (name: string) => {
    const f = await api.createFolder({ parent: listing?.path ?? '', name });
    void qc.invalidateQueries({ queryKey: hostKeys.folders });
    return f;
  };
  return (
    <Dialog
      onClose={onClose}
      title={title}
      size="lg"
      footer={<DialogActions onCancel={onClose} action="Choose" disabled={!selection} onAction={() => selection && onChoose(selection)} />}
    >
      <FolderPicker
        listing={listing}
        loading={q.isFetching}
        error={q.isError ? q.error.message : null}
        onBrowse={onBrowse}
        showHidden={hidden}
        onShowHiddenChange={setHidden}
        onCreateFolder={create}
        onSelectionChange={setSelection}
        autoFocus={!phone}
      />
      <p className="mt-2 truncate text-xs text-muted-foreground" aria-live="polite">
        {selection ? <>Choose <span className="font-mono text-foreground">{selection.display}</span></> : ' '}
      </p>
    </Dialog>
  );
}
