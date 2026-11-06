// A docs page's meta description (search results, link previews): its first paragraph as plain text. The docs are
// plain Markdown without front matter, so content.config.ts sets it from here.

/** Search results show about 160 characters; a longer paragraph ends at a sentence, or a word, before that. */
const MAX = 160;

/** The first paragraph after the page's `# heading`, without Markdown, cut to fit a search result. */
export function docDescription(markdown) {
  const paragraph = [];
  let fenced = false;
  for (const line of markdown.split(/\r?\n/)) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    if (fenced || /^\s*(```|~~~)/.test(line)) continue;
    const text = line.trim();
    if (!text) {
      if (paragraph.length) break;
      continue;
    }
    // Headings, lists, quotes, tables, HTML and images aren't a paragraph.
    if (/^(#|[-*+] |\d+\. |>|\||<|!\[)/.test(text)) {
      if (paragraph.length) break;
      continue;
    }
    paragraph.push(text);
  }
  return shorten(plain(paragraph.join(' ')));
}

function plain(text) {
  return text
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/(\*\*|__)(.+?)\1/g, '$2')
    .replace(/(^|[^\w*])[*_]([^*_]+)[*_](?![\w*])/g, '$1$2')
    .replace(/\s+/g, ' ')
    .trim();
}

function shorten(text) {
  if (text.length <= MAX) return text;
  const head = text.slice(0, MAX);
  const sentence = head.search(/[.!?](?=\s[^.!?]*$)/);
  if (sentence >= 60) return head.slice(0, sentence + 1);
  return `${head.slice(0, head.lastIndexOf(' ')).replace(/[,;:]$/, '')}…`;
}
