import { StrictMode, useState, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { applyThemePref, readThemePref, useThemePref, type ThemePref } from '@ui';
import '../index.css';

/** One gallery section per component group (web/src/gallery/sections/*.tsx). */
export interface Section {
  title: string;
  order: number;
  render: () => ReactNode;
}

const modules = import.meta.glob<{ section: Section }>('./sections/*.tsx', { eager: true });
const sections = Object.values(modules)
  .map((m) => m.section)
  .sort((a, b) => a.order - b.order || a.title.localeCompare(b.title));

function Gallery() {
  const [theme, setTheme] = useThemePref();
  const [phone, setPhone] = useState(() => new URLSearchParams(location.search).has('phone'));
  return (
    <div className="min-h-full bg-background text-foreground">
      <header className="sticky top-0 z-(--pdt-z-popover) flex flex-wrap items-center gap-3 border-b border-border bg-background px-4 py-2">
        <h1 className="text-sm font-semibold">PDT component gallery</h1>
        <span className="text-meta text-muted-foreground">web/ui · dev only</span>
        <span className="flex-1" />
        <label className="flex items-center gap-1 text-xs">
          Theme
          <select aria-label="Theme" value={theme} onChange={(e) => setTheme(e.target.value as ThemePref)}
            className="rounded-md border border-input bg-background px-1 text-xs">
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </label>
        <label className="flex items-center gap-1 text-xs">
          <input type="checkbox" checked={phone} onChange={(e) => setPhone(e.target.checked)} />
          Phone width
        </label>
      </header>
      <main className={`mx-auto flex flex-col gap-6 p-4 ${phone ? 'max-w-[390px]' : 'max-w-6xl'}`}>
        {sections.length === 0 && <p className="text-sm text-muted-foreground">No sections yet.</p>}
        {sections.map((s) => (
          <section key={s.title} aria-label={s.title} className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
            <h2 className="text-sm font-semibold">{s.title}</h2>
            {s.render()}
          </section>
        ))}
      </main>
    </div>
  );
}

applyThemePref(readThemePref());
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={new QueryClient()}>
      <Gallery />
    </QueryClientProvider>
  </StrictMode>,
);
