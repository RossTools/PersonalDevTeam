import { VIEWS, type ViewId } from './views';

export function IconRail({ active, onSelect }: { active: ViewId; onSelect: (v: ViewId) => void }) {
  return (
    <nav aria-label="Views" className="flex md:flex-col gap-1 p-1.5 bg-surface border-border md:border-r border-t md:border-t-0 justify-around md:justify-start">
      {VIEWS.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type="button"
          aria-label={label}
          title={label}
          aria-current={active === id ? 'page' : undefined}
          onClick={() => onSelect(id)}
          className={`p-2 rounded-pdt outline-none focus-visible:ring-2 focus-visible:ring-ring ${active === id ? 'bg-accent-soft text-fg' : 'text-muted hover:text-fg hover:bg-surface-2'}`}
        >
          <Icon size={18} aria-hidden />
        </button>
      ))}
    </nav>
  );
}
