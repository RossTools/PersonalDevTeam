import { memo } from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeSanitize from 'rehype-sanitize';
import { cn } from '@ui/lib/utils';

interface MdNode {
  type: string;
  value?: string;
  children?: MdNode[];
}

/** Turns raw HTML in the source into plain text, so `<Foo>` in a ticket shows as written instead of vanishing. */
function remarkHtmlAsText() {
  const walk = (node: MdNode) => {
    if (node.type === 'html') node.type = 'text';
    node.children?.forEach(walk);
  };
  return (tree: MdNode) => walk(tree);
}

const isExternal = (href: string | undefined) => !!href && !href.startsWith('#');

// Sized for a ticket body: headings step from text-base down to text-xs, never page-title large.
const components: Components = {
  h1: ({ node: _n, ...p }) => <h1 {...p} className="mt-4 mb-2 text-base font-semibold first:mt-0" />,
  h2: ({ node: _n, ...p }) => <h2 {...p} className="mt-4 mb-2 text-sm font-semibold first:mt-0" />,
  h3: ({ node: _n, ...p }) => <h3 {...p} className="mt-3 mb-1.5 text-sm font-medium first:mt-0" />,
  h4: ({ node: _n, ...p }) => <h4 {...p} className="mt-3 mb-1 text-xs font-semibold first:mt-0" />,
  h5: ({ node: _n, ...p }) => <h5 {...p} className="mt-3 mb-1 text-xs font-semibold text-muted-foreground first:mt-0" />,
  h6: ({ node: _n, ...p }) => <h6 {...p} className="mt-3 mb-1 text-xs font-medium text-muted-foreground first:mt-0" />,
  p: ({ node: _n, ...p }) => <p {...p} className="my-2 first:mt-0 last:mb-0" />,
  a: ({ node: _n, href, ...p }) => (
    <a
      {...p}
      href={href}
      target={isExternal(href) ? '_blank' : undefined}
      rel={isExternal(href) ? 'noreferrer' : undefined}
      className="text-primary underline-offset-2 hover:underline focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
    />
  ),
  ul: ({ node: _n, className, ...p }) => (
    <ul {...p} className={cn('my-2 pl-5', className?.includes('contains-task-list') ? 'list-none pl-1' : 'list-disc')} />
  ),
  ol: ({ node: _n, ...p }) => <ol {...p} className="my-2 list-decimal pl-5" />,
  li: ({ node: _n, className, ...p }) => (
    <li {...p} className={cn('my-0.5', className?.includes('task-list-item') && 'list-none')} />
  ),
  input: ({ node: _n, ...p }) => <input {...p} disabled className="mr-1.5 align-[-2px] accent-primary" />,
  blockquote: ({ node: _n, ...p }) => (
    <blockquote {...p} className="my-2 border-l-2 border-border pl-3 text-muted-foreground" />
  ),
  hr: ({ node: _n, ...p }) => <hr {...p} className="my-3 border-border" />,
  pre: ({ node: _n, ...p }) => (
    <pre
      {...p}
      className="my-2 overflow-x-auto rounded-md border border-border bg-surface-2 p-2 font-mono text-xs leading-relaxed text-foreground [&>code]:bg-transparent [&>code]:p-0 [&>code]:text-xs"
    />
  ),
  code: ({ node: _n, ...p }) => <code {...p} className="rounded-sm bg-muted px-1 py-px font-mono text-meta text-foreground" />,
  table: ({ node: _n, ...p }) => (
    <div className="my-2 overflow-x-auto">
      <table {...p} className="border-collapse text-xs" />
    </div>
  ),
  th: ({ node: _n, ...p }) => <th {...p} className="border border-border bg-muted px-2 py-1 text-left font-medium" />,
  td: ({ node: _n, ...p }) => <td {...p} className="border border-border px-2 py-1 align-top" />,
  del: ({ node: _n, ...p }) => <del {...p} className="text-muted-foreground" />,
  img: ({ node: _n, alt, ...p }) => <img {...p} alt={alt ?? ''} className="my-2 max-w-full rounded-md" />,
};

/** A Markdown file by its name (the Files view and the review show these rendered on request). */
export const isMarkdownPath = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

export interface MarkdownProps {
  /** The Markdown source (GitHub-flavoured: tables, task lists, strikethrough, autolinks). */
  children: string;
  className?: string;
}

/**
 * Renders user-written Markdown (ticket descriptions, comments). Raw HTML is never rendered: it shows as the text it
 * was written as, and rehype-sanitize (GitHub's schema) strips anything unsafe that remains (`javascript:` links). Links open in a new tab.
 */
export const Markdown = memo(function Markdown({ children, className }: MarkdownProps) {
  return (
    <div data-slot="markdown" className={cn('min-w-0 text-sm leading-relaxed break-words text-foreground', className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm, remarkHtmlAsText]} rehypePlugins={[rehypeSanitize]} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
});
