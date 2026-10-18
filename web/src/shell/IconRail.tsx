import { cn } from '@ui/lib/utils';
import { IconButton } from '@ui';
import { VIEWS, type ViewId } from './views';

export function IconRail({ active, onSelect }: { active: ViewId; onSelect: (v: ViewId) => void }) {
  return (
    <nav aria-label="Views" className="flex justify-around gap-1 border-t border-border bg-card p-1.5 md:flex-col md:justify-start md:border-t-0 md:border-r">
      {VIEWS.map(({ id, label, icon: Icon }) => (
        <IconButton
          key={id}
          label={label}
          aria-current={active === id ? 'page' : undefined}
          onClick={() => onSelect(id)}
          className={cn('size-9 pointer-coarse:size-11', active === id ? 'bg-accent text-foreground' : 'text-muted-foreground')}
        >
          <Icon aria-hidden />
        </IconButton>
      ))}
    </nav>
  );
}
