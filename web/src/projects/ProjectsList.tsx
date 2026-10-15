import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';

export function ProjectsList({ selectedId, onSelect }: { selectedId: string | null; onSelect: (id: string) => void }) {
  const q = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  if (q.isPending) return <p className="px-2 text-xs text-muted">Loading…</p>;
  if (q.isError) return <p role="alert" className="px-2 text-xs text-failed">{q.error.message}</p>;
  if (q.data.length === 0) return <p className="px-2 text-xs text-muted">No projects yet.</p>;
  return (
    <ul className="flex flex-col gap-0.5">
      {q.data.map((p) => (
        <li key={p.id}>
          <button
            type="button"
            aria-current={selectedId === p.id ? 'true' : undefined}
            onClick={() => onSelect(p.id)}
            className={`w-full text-left px-2 py-1 rounded-pdt text-sm truncate ${selectedId === p.id ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
            title={p.repoPath}
          >
            {p.name}
          </button>
        </li>
      ))}
    </ul>
  );
}
