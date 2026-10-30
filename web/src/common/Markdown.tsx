import { memo, useContext, useMemo, type ComponentType } from 'react';
import ReactMarkdown, { type Components, type ExtraProps } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import rehypeSanitize from 'rehype-sanitize';
import { Check } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { TicketKeysContext } from './ticketKeys';
import { AttachmentImage, AttachmentLink, attachmentRef } from './AttachmentViews';

interface MdNode {
  type: string;
  value?: string;
  url?: string;
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

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Turns ticket keys of the given projects in text into links (not inside links or code, which are not text nodes). */
export function remarkTicketKeys(keys: readonly string[]) {
  const re = new RegExp(`\\b(?:${keys.map(escapeRe).join('|')})-\\d+\\b`, 'g');
  const split = (value: string): MdNode[] => {
    const out: MdNode[] = [];
    let at = 0;
    for (const m of value.matchAll(re)) {
      if (m.index > at) out.push({ type: 'text', value: value.slice(at, m.index) });
      out.push({ type: 'link', url: `/tickets/${m[0]}`, children: [{ type: 'text', value: m[0] }] });
      at = m.index + m[0].length;
    }
    if (at < value.length) out.push({ type: 'text', value: value.slice(at) });
    return out;
  };
  const walk = (node: MdNode) => {
    if (!node.children || node.type === 'link' || node.type === 'linkReference') return;
    node.children = node.children.flatMap((c) => {
      if (c.type === 'text' && c.value) return split(c.value);
      walk(c);
      return [c];
    });
  };
  return () => (tree: MdNode) => walk(tree);
}

/** A link that leaves the app (a new tab), not one of its own paths or an anchor. */
const isExternal = (href: string | undefined) => !!href && !href.startsWith('#') && !(href.startsWith('/') && !href.startsWith('//'));

// Sized for a ticket body: headings step from text-base down to text-xs, never page-title large.
const components: Components = {
  h1: ({ node: _n, ...p }) => <h1 {...p} className="mt-4 mb-2 text-base font-semibold first:mt-0" />,
  h2: ({ node: _n, ...p }) => <h2 {...p} className="mt-4 mb-2 text-sm font-semibold first:mt-0" />,
  h3: ({ node: _n, ...p }) => <h3 {...p} className="mt-3 mb-1.5 text-sm font-medium first:mt-0" />,
  h4: ({ node: _n, ...p }) => <h4 {...p} className="mt-3 mb-1 text-xs font-semibold first:mt-0" />,
  h5: ({ node: _n, ...p }) => <h5 {...p} className="mt-3 mb-1 text-xs font-semibold text-muted-foreground first:mt-0" />,
  h6: ({ node: _n, ...p }) => <h6 {...p} className="mt-3 mb-1 text-xs font-medium text-muted-foreground first:mt-0" />,
  p: ({ node: _n, ...p }) => <p {...p} className="my-2 first:mt-0 last:mb-0" />,
  // A ticket's attached text file opens in a viewer (PDT-300).
  a: ({ node: _n, href, ...p }) => attachmentRef(href) ? (
    <AttachmentLink src={href!} name={typeof p.children === 'string' ? p.children : 'Attachment'}>{p.children}</AttachmentLink>
  ) : (
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
  // A ticket's attached image opens full size (PDT-300).
  img: ({ node: _n, alt, ...p }) => typeof p.src === 'string' && attachmentRef(p.src)
    ? <AttachmentImage src={p.src} alt={alt ?? ''} />
    : <img {...p} alt={alt ?? ''} className="my-2 max-w-full rounded-md" />,
};

// A document to read (a note, PDT-143): roomier, with headings a step above the body and quieter list markers.
const documentComponents: Components = {
  ...components,
  h1: ({ node: _n, ...p }) => <h1 {...p} className="mt-8 mb-2 text-base font-semibold tracking-tight first:mt-0" />,
  h2: ({ node: _n, ...p }) => <h2 {...p} className="mt-7 mb-2 text-base font-semibold tracking-tight first:mt-0" />,
  h3: ({ node: _n, ...p }) => <h3 {...p} className="mt-6 mb-2 text-base font-semibold tracking-tight first:mt-0" />,
  h4: ({ node: _n, ...p }) => <h4 {...p} className="mt-5 mb-1 text-sm font-semibold first:mt-0" />,
  p: ({ node: _n, ...p }) => <p {...p} className="my-3.5 first:mt-0 last:mb-0" />,
  ul: ({ node: _n, className, ...p }) => (
    <ul {...p} className={cn('my-3.5 marker:text-muted-foreground', className?.includes('contains-task-list') ? 'list-none pl-0' : 'list-disc pl-6')} />
  ),
  ol: ({ node: _n, ...p }) => <ol {...p} className="my-3.5 list-decimal pl-6 marker:text-muted-foreground" />,
  li: ({ node: _n, className, ...p }) => (
    <li
      {...p}
      className={cn('my-1 pl-1', className?.includes('task-list-item') && 'list-none pl-0 has-[[aria-checked=true]]:text-muted-foreground has-[[aria-checked=true]]:line-through')}
    />
  ),
  // A task's box, for show only (a disabled native checkbox greys out): ticked ones fill with the accent, their text
  // muted and struck through.
  input: ({ node: _n, checked }) => (
    <span
      role="checkbox"
      aria-checked={!!checked}
      aria-disabled
      className={cn(
        'mr-2.5 inline-grid size-4 place-items-center rounded-sm border-[1.5px] align-[-3px]',
        checked ? 'border-primary bg-primary text-primary-foreground' : 'border-muted-foreground',
      )}
    >
      {checked && <Check aria-hidden className="size-3" strokeWidth={3} />}
    </span>
  ),
  code: ({ node: _n, ...p }) => <code {...p} className="rounded-sm bg-surface-2 px-1.5 py-0.5 font-mono text-sm text-foreground" />,
  blockquote: ({ node: _n, ...p }) => (
    <blockquote {...p} className="my-3.5 border-l-[3px] border-border pl-4 text-muted-foreground" />
  ),
  pre: ({ node: _n, ...p }) => (
    <pre
      {...p}
      className="my-3.5 overflow-x-auto rounded-md border border-border bg-surface-2 p-3 font-mono text-sm leading-relaxed text-foreground [&>code]:bg-transparent [&>code]:p-0"
    />
  ),
  table: ({ node: _n, ...p }) => (
    <div className="my-3.5 overflow-x-auto">
      <table {...p} className="border-collapse text-sm" />
    </div>
  ),
};

// Each block of a document says where its source starts (`data-source-start`, an offset into the text), so a
// double-click on it can open the editor there (a note, PDT-187).
const SOURCED = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'li', 'blockquote', 'pre', 'table'] as const;
type Block = ComponentType<ExtraProps & Record<string, unknown>>;
const sourced = documentComponents as Record<string, Block>;
for (const tag of SOURCED) {
  const Inner = sourced[tag];
  sourced[tag] = (props) => <Inner {...props} data-source-start={props.node?.position?.start.offset} />;
}

const plugins = [remarkGfm, remarkHtmlAsText];
const withBreaks = [...plugins, remarkBreaks];

/** A Markdown file by its name (the Files view and the review show these rendered on request). */
export const isMarkdownPath = (path: string) => /\.(md|markdown|mdx)$/i.test(path);

export interface MarkdownProps {
  /** The Markdown source (GitHub-flavoured: tables, task lists, strikethrough, autolinks). */
  children: string;
  /** Keep single line breaks as breaks, as GitHub issues do (ticket descriptions: pasted logs or JSON without a code
   *  fence would otherwise run together). Off for Markdown files, whose lines are wrapped by hand. */
  breaks?: boolean;
  /** `document`: a page to read (a note): body text-base with a roomy line height, headings a step above it. */
  variant?: 'default' | 'document';
  className?: string;
}

/**
 * Renders user-written Markdown (ticket descriptions, comments). Raw HTML is never rendered: it shows as the text it
 * was written as, and rehype-sanitize (GitHub's schema) strips anything unsafe that remains (`javascript:` links). Links open in a new tab,
 * except the app's own paths (ticket keys, PDT-111), which the app navigates to.
 */
export const Markdown = memo(function Markdown({ children, breaks, variant = 'default', className }: MarkdownProps) {
  const keys = useContext(TicketKeysContext);
  const keyList = keys.join(' ');
  const remarkPlugins = useMemo(() => {
    const base = breaks ? withBreaks : plugins;
    return keyList ? [...base, remarkTicketKeys(keyList.split(' '))] : base;
  }, [breaks, keyList]);
  return (
    <div
      data-slot="markdown"
      data-variant={variant}
      className={cn('min-w-0 text-sm leading-relaxed break-words text-foreground', variant === 'document' && 'text-base leading-[1.7]', className)}
    >
      <ReactMarkdown remarkPlugins={remarkPlugins} rehypePlugins={[rehypeSanitize]} components={variant === 'document' ? documentComponents : components}>
        {children}
      </ReactMarkdown>
    </div>
  );
});
