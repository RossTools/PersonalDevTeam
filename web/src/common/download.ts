/** Saves what pdtd serves at `url` through the browser's download (pdtd names the file). */
export function download(url: string) {
  const a = document.createElement('a');
  a.href = url;
  a.download = '';
  document.body.append(a);
  a.click();
  a.remove();
}

/** An attachment's URL that saves it under its name rather than showing it. */
export const downloadUrl = (src: string) => `${src}${src.includes('?') ? '&' : '?'}download=1`;
