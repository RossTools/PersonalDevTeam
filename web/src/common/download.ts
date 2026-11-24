/** Clicks a link to `href` that downloads; `name` names the file (empty: the server names it). */
function save(href: string, name: string) {
  const a = document.createElement('a');
  a.href = href;
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
}

/** Saves what pdtd serves at `url` through the browser's download (pdtd names the file). */
export function download(url: string) {
  save(url, '');
}

/** Saves `text` (JSON) as the file `name` through the browser's download. */
export function downloadText(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  save(url, name);
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

/** An attachment's URL that saves it under its name rather than showing it. */
export const downloadUrl = (src: string) => `${src}${src.includes('?') ? '&' : '?'}download=1`;
