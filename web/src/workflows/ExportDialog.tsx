// Export… (PDT-155): the export's text to select and copy, with Download beside it. From the Workflows list it first
// asks which workflows go in (all of them, to start with).
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, Skeleton, Textarea } from '@ui';
import type { Workflow } from '../api/gen/Workflow';
import { copyToClipboard } from '../common/clipboard';
import { downloadText, exportFileName, exportText, fetchExport } from './transfer';
import { workflowKeys } from './queries';

function ExportText({ ids }: { ids: string[] }) {
  const e = useQuery({ queryKey: workflowKeys.export(ids), queryFn: () => fetchExport(ids), staleTime: Infinity, gcTime: 0 });
  if (e.error) return <Alert tone="error">The export could not be made: {e.error.message}</Alert>;
  if (!e.data) return <Skeleton lines={6} />;
  const text = exportText(e.data);
  const data = e.data;
  return (
    <div className="flex flex-col gap-2">
      <Textarea mono readOnly rows={14} aria-label="Export" value={text} className="max-h-[50vh] text-xs"
        onFocus={(ev) => ev.target.select()} />
      <div className="flex justify-end gap-2">
        <Button variant="secondary" onClick={() => downloadText(exportFileName(data), text)}>Download</Button>
        <Button variant="primary" onClick={() => void copyToClipboard(text, 'Copied')}>Copy</Button>
      </div>
    </div>
  );
}

export function ExportDialog({ ids, workflows, onClose }: {
  /** The workflows to export; without it, `workflows` are offered to pick from. */
  ids?: string[];
  workflows?: Workflow[];
  onClose: () => void;
}) {
  const [picked, setPicked] = useState<string[]>(() => (workflows ?? []).map((w) => w.id));
  const [shown, setShown] = useState<string[] | null>(ids ?? null);
  const description = 'Paste it into Import on another PDT, or save it as a file. Secrets, runs and state stay here.';
  return (
    <Dialog open onOpenChange={(o) => { if (!o) onClose(); }} title="Export workflows" description={description} size="lg"
      footer={shown ? <Button variant="secondary" onClick={onClose}>Close</Button> : (
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={picked.length === 0} onClick={() => setShown((workflows ?? []).filter((w) => picked.includes(w.id)).map((w) => w.id))}>Export {picked.length}</Button>
        </>
      )}>
      {shown ? <ExportText ids={shown} /> : (
        <fieldset className="flex flex-col gap-1.5">
          <legend className="sr-only">Workflows to export</legend>
          <Checkbox label="All" checked={picked.length === (workflows ?? []).length}
            indeterminate={picked.length > 0 && picked.length < (workflows ?? []).length}
            onChange={(on) => setPicked(on ? (workflows ?? []).map((w) => w.id) : [])} />
          {(workflows ?? []).map((w) => (
            <Checkbox key={w.id} label={w.name} checked={picked.includes(w.id)}
              onChange={(on) => setPicked((p) => (on ? [...p, w.id] : p.filter((x) => x !== w.id)))} />
          ))}
        </fieldset>
      )}
    </Dialog>
  );
}
