/** A Markdown file by its name (the Files view and the review show these rendered on request). */
export const isMarkdownPath = (path: string) => /\.(md|markdown|mdx)$/i.test(path);
