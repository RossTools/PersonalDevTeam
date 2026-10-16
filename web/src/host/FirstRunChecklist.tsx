import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { checklist } from './checklist';

export function FirstRunChecklist({ onOpenSettings }: { onOpenSettings?: () => void }) {
  const settings = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const update = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus });
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks });
  if (!settings.data) return null;
  const items = checklist(settings.data, update.data, checks.data);
  if (!items.length) return null;
  return (
    <section aria-label="Set up this host" className="m-2 rounded-pdt border border-border bg-surface p-2 text-xs">
      <h3 className="font-semibold mb-1">Set up this host</h3>
      <ul className="list-disc pl-4 text-muted">
        {items.map((i) => <li key={i.id}>{i.text}</li>)}
      </ul>
      {onOpenSettings && (
        <button type="button" onClick={onOpenSettings} className="mt-1 text-accent underline">Open settings</button>
      )}
    </section>
  );
}
