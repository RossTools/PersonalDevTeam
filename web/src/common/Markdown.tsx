import { createContext, memo, useContext, useMemo, type ComponentProps, type ComponentType, type ImgHTMLAttributes, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import ReactMarkdown, { type Components, type ExtraProps } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import rehypeSanitize from 'rehype-sanitize';
import { Check } from 'lucide-react';
import { Badge } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import { fileHref, filePathOf, isSha, MarkdownRefsContext, prNumberOf, prUrl, statusTone, type MarkdownRefs } from './markdownRefs';
import { PrPill, TicketPill } from './TicketPill';
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
  return remarkRefs(keys, null);
}

/**
 * Ticket keys in text become links to the ticket (not inside links or code, which are not text nodes). In a
 * conversation (`refs`, PDT-382) also: `#12` a link to the forge's PR, a commit SHA inline code (shown muted), and a
 * file path in inline code a link to the file viewer (shown only if the file exists).
 */
export function remarkRefs(keys: readonly string[], refs: MarkdownRefs | null) {
  const parts = [
    keys.length ? `\\b(?:${keys.map(escapeRe).join('|')})-\\d+\\b` : null,
    refs && prUrl(refs.forge, 1) ? '(?:\\bPR )?(?<![\\w&/#])#\\d+\\b' : null,
    refs ? '\\b[0-9a-f]{7,40}\\b' : null,
  ].filter(Boolean);
  const re = parts.length ? new RegExp(parts.join('|'), 'g') : null;
  const ref = (m: string): MdNode | null => {
    // `#12`, or `PR #12` (the pill says "PR #12" either way).
    if (m.includes('#')) {
      const n = Number(m.slice(m.indexOf('#') + 1));
      const url = prUrl(refs!.forge, n);
      return url ? { type: 'link', url, children: [{ type: 'text', value: `#${n}` }] } : null;
    }
    if (refs && /^[0-9a-f]+$/.test(m)) return isSha(m) ? { type: 'inlineCode', value: m } : null;
    return { type: 'link', url: `/tickets/${m}`, children: [{ type: 'text', value: m }] };
  };
  const split = (value: string): MdNode[] => {
    if (!re) return [{ type: 'text', value }];
    const out: MdNode[] = [];
    let at = 0;
    for (const m of value.matchAll(re)) {
      const node = ref(m[0]);
      if (!node) continue;
      if (m.index > at) out.push({ type: 'text', value: value.slice(at, m.index) });
      out.push(node);
      at = m.index + m[0].length;
    }
    if (at < value.length) out.push({ type: 'text', value: value.slice(at) });
    return out;
  };
  const walk = (node: MdNode) => {
    if (!node.children || node.type === 'link' || node.type === 'linkReference') return;
    node.children = node.children.flatMap((c) => {
      if (c.type === 'text' && c.value) return split(c.value);
      const path = refs?.sessionId && c.type === 'inlineCode' && c.value ? filePathOf(c.value) : null;
      if (path) return [{ type: 'link', url: fileHref(refs!, path), children: [c] }];
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
  a: ({ node: _n, ...p }) => <MdLink {...p} />,
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
  code: ({ node: _n, ...p }) => <MdCode {...p} />,
  table: ({ node: _n, ...p }) => <MdTable {...p} />,
  th: ({ node: _n, ...p }) => <MdCell head {...p} />,
  td: ({ node: _n, ...p }) => <MdCell {...p} />,
  del: ({ node: _n, ...p }) => <del {...p} className="text-muted-foreground" />,
  // A ticket's attached image opens full size (PDT-300).
  img: ({ node: _n, alt, ...p }) => typeof p.src === 'string' && attachmentRef(p.src)
    ? <AttachmentImage src={p.src} alt={alt ?? ''} />
    : <ResolvedImage {...p} alt={alt ?? ''} />,
};

const LINK = 'text-primary underline-offset-2 hover:underline focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none';

/** A ticket's path (`/tickets/PDT-12`) → its key. */
const ticketOfHref = (href: string | undefined) => (href ? /^\/tickets\/([A-Za-z][A-Za-z0-9]*-\d+)$/.exec(href)?.[1] ?? null : null);

/**
 * A link: an attachment opens in its viewer (PDT-300); in a conversation a ticket key or a PR is a pill and a file
 * path opens the file viewer (PDT-382); the rest are links (a new tab when they leave the app).
 */
function MdLink({ href, children, ...p }: ComponentProps<'a'>) {
  const refs = useContext(MarkdownRefsContext);
  if (attachmentRef(href)) {
    return <AttachmentLink src={href!} name={typeof children === 'string' ? children : 'Attachment'}>{children}</AttachmentLink>;
  }
  if (refs) {
    const key = ticketOfHref(href);
    if (key && children === key) return <TicketPill ticketKey={key} />;
    const pr = prNumberOf(refs.forge, href);
    if (pr !== null && (children === `#${pr}` || children === href)) return <PrPill number={pr} href={href!} />;
    if (href?.startsWith('/files/') && refs.sessionId) return <FileLink href={href} sessionId={refs.sessionId}>{children}</FileLink>;
  }
  return (
    <a {...p} href={href} target={isExternal(href) ? '_blank' : undefined} rel={isExternal(href) ? 'noreferrer' : undefined} className={LINK}>
      {children}
    </a>
  );
}

/** A file path an agent wrote, as a link to the file viewer once the session's folder has it (as written otherwise). */
function FileLink({ href, sessionId, children }: { href: string; sessionId: string; children: ReactNode }) {
  const path = decodeURIComponent(href.slice('/files/'.length).split('?')[0]);
  const exists = useQuery({
    queryKey: ['session-file', sessionId, path],
    queryFn: async () => (await api.sessionFiles(sessionId, path)).includes(path),
    staleTime: 5 * 60_000,
    retry: false,
  });
  if (!exists.data) return <>{children}</>;
  return (
    <a href={href} title={`Open ${path}`} data-slot="file-link" className="underline decoration-border underline-offset-2 hover:decoration-primary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none [&>code]:text-primary">
      {children}
    </a>
  );
}

/** Inline code; in a conversation a commit SHA is muted mono (PDT-382). */
function MdCode(p: ComponentProps<'code'>) {
  const refs = useContext(MarkdownRefsContext);
  return refs && typeof p.children === 'string' && isSha(p.children)
    ? <code {...p} className="font-mono text-meta text-muted-foreground" />
    : <code {...p} className="rounded-sm bg-muted px-1 py-px font-mono text-meta text-foreground" />;
}

/** A table; in a conversation framed as a card (PDT-382). */
function MdTable(p: ComponentProps<'table'>) {
  const refs = useContext(MarkdownRefsContext);
  if (!refs) {
    return (
      <div className="my-2 overflow-x-auto">
        <table {...p} className="border-collapse text-xs" />
      </div>
    );
  }
  return (
    <div data-slot="table-card" className="my-2 overflow-x-auto rounded-lg border border-border bg-card">
      <table {...p} className="w-full border-collapse text-xs [&_tr:last-child>td]:border-b-0" />
    </div>
  );
}

/** A table cell; in a conversation a known status word in one is a coloured chip ("CI failed", "ready to accept"). */
function MdCell({ head, children, ...p }: ComponentProps<'td'> & { head?: boolean }) {
  const refs = useContext(MarkdownRefsContext);
  if (head) {
    return refs
      ? <th {...(p as ComponentProps<'th'>)} className="border-b border-border px-2.5 py-1.5 text-left font-medium text-muted-foreground">{children}</th>
      : <th {...(p as ComponentProps<'th'>)} className="border border-border bg-muted px-2 py-1 text-left font-medium">{children}</th>;
  }
  if (!refs) return <td {...p} className="border border-border px-2 py-1 align-top">{children}</td>;
  const tone = typeof children === 'string' ? statusTone(children) : null;
  return (
    <td {...p} className="border-b border-border px-2.5 py-1.5 align-top">
      {tone ? <Badge tone={tone}>{children}</Badge> : children}
    </td>
  );
}

/** Where a relative image of the Markdown on show is served from (a local doc's images sit beside it, PDT-315). */
export const MarkdownImageContext = createContext<((src: string) => string) | null>(null);

const isRelative = (src: string) => !/^([a-z][a-z0-9+.-]*:|\/|#)/i.test(src);

function ResolvedImage({ src, ...p }: ImgHTMLAttributes<HTMLImageElement>) {
  const resolve = useContext(MarkdownImageContext);
  const shown = typeof src === 'string' && resolve && isRelative(src) ? resolve(src) : src;
  return <img {...p} src={shown} className="my-2 max-w-full rounded-md" />;
}

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
  const refs = useContext(MarkdownRefsContext);
  const keyList = keys.join(' ');
  const remarkPlugins = useMemo(() => {
    const base = breaks ? withBreaks : plugins;
    return keyList || refs ? [...base, remarkRefs(keyList ? keyList.split(' ') : [], refs)] : base;
  }, [breaks, keyList, refs]);
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
