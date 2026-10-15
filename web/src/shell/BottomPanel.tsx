export function BottomPanel() {
  return (
    <section aria-label="Terminal panel" className="h-56 border-t border-border bg-surface flex flex-col">
      <div className="flex gap-3 px-3 py-1.5 border-b border-border text-xs text-muted">Terminals</div>
      <div className="flex-1 p-3 font-mono text-xs text-muted">No terminal sessions yet.</div>
    </section>
  );
}
