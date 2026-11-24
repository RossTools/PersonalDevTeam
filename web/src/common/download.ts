/** An attachment's URL that saves it under its name rather than showing it. */
export const downloadUrl = (src: string) => `${src}${src.includes('?') ? '&' : '?'}download=1`;
