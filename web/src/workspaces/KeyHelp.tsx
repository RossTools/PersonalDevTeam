import { Button, Dialog, Kbd } from '@ui';
import { KEY_HELP } from './keys';

/** The list of prefix keys (Ctrl+] then one key), and Report a bug when `onReportBug` is given. */
export function KeyHelp({ onClose, onReportBug }: { onClose: () => void; onReportBug?: () => void }) {
  return (
    <Dialog
      onClose={onClose}
      title="Keyboard shortcuts"
      description={<>Press <Kbd keys={['Ctrl', ']']} /> then one key.</>}
      size="sm"
      footer={onReportBug ? <Button variant="secondary" onClick={onReportBug}>Report a bug…</Button> : undefined}
    >
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        {KEY_HELP.map(([k, d]) => (
          <div key={k} className="contents">
            <dt><Kbd>{k}</Kbd></dt>
            <dd className="text-muted-foreground">{d}</dd>
          </div>
        ))}
      </dl>
    </Dialog>
  );
}
