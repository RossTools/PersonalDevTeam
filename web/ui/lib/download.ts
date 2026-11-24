/**
 * Saves a URL (a server picks the file's name unless `name` does) or a Blob (named `name`) through the browser's
 * download. In render, a link that saves something is `<Link download>` or a `Button`/`IconButton` with `href` and
 * `download`; this is for actions that make the file first.
 */
export function download(what: string | Blob, name = '') {
  const url = typeof what === 'string' ? what : URL.createObjectURL(what);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  if (typeof what !== 'string') setTimeout(() => URL.revokeObjectURL(url), 0);
}
