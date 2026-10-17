import { KEY_HELP } from './keys';

/** The list of prefix keys (Ctrl+] then one key). */
export function KeyHelp({ onClose }: { onClose: () => void }) {
  return (
    <>
      <div className="fixed inset-0 z-10 bg-black/20" onClick={onClose} aria-hidden />
      <div
        role="dialog"
        aria-label="Keyboard shortcuts"
        tabIndex={-1}
        ref={(el) => el?.focus()}
        onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}
        className="absolute z-20 left-2 right-2 md:left-1/2 md:right-auto md:-translate-x-1/2 top-12 md:w-96 rounded-pdt border border-border bg-surface shadow-lg p-4 outline-none"
      >
        <h2 className="text-sm font-medium mb-2">Keyboard shortcuts</h2>
        <p className="text-xs text-muted mb-3">Press Ctrl+] then one key.</p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          {KEY_HELP.map(([k, d]) => (
            <div key={k} className="contents">
              <dt><kbd className="font-mono text-xs rounded border border-border px-1">{k}</kbd></dt>
              <dd className="text-muted">{d}</dd>
            </div>
          ))}
        </dl>
      </div>
    </>
  );
}
