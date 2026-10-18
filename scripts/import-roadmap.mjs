#!/usr/bin/env node
// One-off import of docs/ROADMAP.md into PDT tickets (tickets spec §9). Kept for reference after it has run.
//
//   node scripts/import-roadmap.mjs                       # dry run: prints the tickets it would make
//   node scripts/import-roadmap.mjs --apply --base http://127.0.0.1:7420 --project <project id>
//
// "Next" items become Ready tickets in their listed order; "Follow-ups" become Drafts, also in order. A ticket's title
// is the item's bold lead (or its first sentence), its description the rest. A line inside an item that starts with
// a bold lead begins a ticket of its own. With --apply the tickets are created through the API and the roadmap file
// is replaced by a pointer to PDT. Run it once, with the user's go-ahead.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ROADMAP = join(dirname(fileURLToPath(import.meta.url)), '..', 'docs', 'ROADMAP.md');

/** The items of one `## <heading>` section: `[{ title, description }]`. */
export function parseSection(markdown, heading) {
  const lines = markdown.split('\n');
  const start = lines.findIndex((l) => l.startsWith(`## ${heading}`));
  if (start < 0) return [];
  const body = [];
  for (const l of lines.slice(start + 1)) {
    if (l.startsWith('## ')) break;
    body.push(l);
  }
  // Group into items: a list marker starts one, an indented line continues it, a continuation that opens with a bold
  // lead starts another.
  const items = [];
  for (const raw of body) {
    const marker = raw.match(/^(?:\d+\.|-)\s+(.*)$/);
    const cont = raw.match(/^\s{2,}(\S.*)$/);
    if (marker) items.push([marker[1]]);
    else if (cont && items.length > 0) {
      if (cont[1].startsWith('**')) items.push([cont[1]]);
      else items[items.length - 1].push(cont[1]);
    }
  }
  return items.map((parts) => split(parts.join(' ')));
}

/**
 * `**Lead.** rest` or `**Lead:** rest` → title "Lead", description "rest". Otherwise the lead is part of a sentence
 * (`**Notifications** for unseen sessions.`): the title is the plain text up to the first ` (`, `:` or `.`, and the
 * description is the whole item (unless that adds nothing).
 */
export function split(text) {
  const lead = text.match(/^\*\*(.+?)\*\*\s*(.*)$/);
  if (lead) {
    const closed = /[.:]$/.test(lead[1]) || /^[.:]/.test(lead[2]);
    if (closed) return { title: lead[1].replace(/[.:]\s*$/, '').trim(), description: lead[2].replace(/^[.:]\s*/, '').trim() };
    // A lead that is a title of its own ("Phase 4c: areas and overlap").
    if (lead[1].includes(':')) return { title: lead[1].trim(), description: text.replace(/\*\*/g, '').trim() };
  }
  const plain = text.replace(/\*\*/g, '').trim();
  const title = plain.split(/\s\(|:|\.(?:\s|$)/)[0].replace(/,\s*$/, '').trim();
  const rest = plain.replace(/\.$/, '');
  return { title, description: rest === title ? '' : plain };
}

export const POINTER = `# Roadmap

The roadmap lives in PDT now: the project's tickets (Tickets → Backlog, or the Board for what is in flight). Designs
stay in docs/superpowers/specs, plans in docs/superpowers/plans, known limits in docs/superpowers/notes/*-carryover.md.
`;

async function call(base, method, path, body) {
  const res = await fetch(`${base}${path}`, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
  return res.json();
}

async function main(argv) {
  const arg = (name) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const apply = argv.includes('--apply');
  const md = readFileSync(ROADMAP, 'utf8');
  const groups = [
    { stage: 'ready', items: parseSection(md, 'Next') },
    { stage: 'draft', items: parseSection(md, 'Follow-ups') },
  ];
  for (const g of groups) {
    console.log(`\n${g.stage.toUpperCase()} (${g.items.length})`);
    for (const t of g.items) console.log(`- ${t.title}${t.description ? `\n    ${t.description}` : ''}`);
  }
  if (!apply) {
    console.log('\nDry run. Add --apply --base <url> --project <id> to create these tickets.');
    return;
  }
  const base = arg('--base');
  const project = arg('--project');
  if (!base || !project) throw new Error('--apply needs --base and --project');
  let made = 0;
  // One ranked list: Next first, then the follow-ups below it.
  let after;
  for (const g of groups) {
    for (const t of g.items) {
      // Each one below the previous, so the backlog keeps the roadmap's order.
      const body = { title: t.title, stage: g.stage, ...(t.description ? { description: t.description } : {}), ...(after ? { after } : {}) };
      const created = await call(base, 'POST', `/api/projects/${project}/tickets`, body);
      after = created.id;
      made += 1;
      console.log(`created ${created.key} ${t.title}`);
    }
  }
  writeFileSync(ROADMAP, POINTER);
  console.log(`\n${made} tickets created; docs/ROADMAP.md now points to PDT.`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
