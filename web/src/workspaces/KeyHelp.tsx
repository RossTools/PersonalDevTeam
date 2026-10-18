import { Dialog, Kbd } from '@ui';
import { KEY_HELP } from './keys';

/** The list of prefix keys (Ctrl+] then one key). */
export function KeyHelp({ onClose }: { onClose: () => void }) {
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title="Keyboard shortcuts"
      description={<>Press <Kbd keys={['Ctrl', ']']} /> then one key.</>}
      size="sm"
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
