import { useState } from 'react';
import { FolderPicker, type FolderPickerFolder, type FolderPickerListing } from '@ui/FolderPicker';

// A fake host for the picker: a macOS home folder and a Windows one, so both path forms can be seen.
function host(sep: string, homeRoot: string, places: { name: string; path: string }[]): Record<string, FolderPickerListing> {
  const j = (...p: string[]) => p.join(sep);
  const home = homeRoot;
  const f = (path: string, display: string, extra: Partial<FolderPickerFolder> = {}): FolderPickerFolder => ({
    name: path.split(sep).pop()!, path, display, ...extra,
  });
  const listing = (path: string, display: string, crumbs: string[], folders: FolderPickerFolder[]): FolderPickerListing => ({
    path, display, parent: path === home ? home.slice(0, home.lastIndexOf(sep)) || sep : home, sep, places, folders,
    crumbs: [{ name: '~', path: home }, ...crumbs.map((c) => ({ name: c, path: j(home, c) }))],
  });
  return {
    [home]: listing(home, '~', [], [
      f(j(home, '.config'), `~${sep}.config`, { hidden: true }),
      f(j(home, 'Documents'), `~${sep}Documents`),
      f(j(home, 'work'), `~${sep}work`),
    ]),
    [j(home, 'work')]: listing(j(home, 'work'), `~${sep}work`, ['work'], [
      f(j(home, 'work', 'PersonalDevTeam'), `~${sep}work${sep}PersonalDevTeam`, { git: true, project: { name: 'PDT', key: 'PDT' } }),
      f(j(home, 'work', 'sample-app'), `~${sep}work${sep}sample-app`, { git: true }),
      f(j(home, 'work', 'scratch'), `~${sep}work${sep}scratch`),
    ]),
  };
}

const MAC = host('/', '/Users/me', [{ name: 'Home', path: '/Users/me' }, { name: 'Projects folder', path: '/Users/me/work' }, { name: '/', path: '/' }]);
const WIN = host('\\', 'C:\\Users\\me', [{ name: 'Home', path: 'C:\\Users\\me' }, { name: 'C:', path: 'C:\\' }, { name: 'D:', path: 'D:\\' }]);

function Demo({ label, fs, start }: { label: string; fs: Record<string, FolderPickerListing>; start: string }) {
  const [path, setPath] = useState(start);
  const [hidden, setHidden] = useState(false);
  const raw = fs[path];
  const listing = raw && { ...raw, folders: raw.folders.filter((f) => hidden || !f.hidden) };
  return (
    <div className="flex max-w-xl flex-col gap-1.5">
      <div className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
      <FolderPicker
        listing={listing ?? fs[start]}
        error={listing ? null : `${path} does not exist (in this demo only ~ and ~/work do)`}
        onBrowse={setPath}
        showHidden={hidden}
        onShowHiddenChange={setHidden}
        onCreateFolder={async (name) => { throw new Error(`demo: would create ${name}`); }}
      />
    </div>
  );
}

function render() {
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <Demo label="FolderPicker · macOS host" fs={MAC} start="/Users/me/work" />
      <Demo label="FolderPicker · Windows host" fs={WIN} start="C:\Users\me" />
    </div>
  );
}

export const section = { title: 'Folder picker', order: 42, render };
