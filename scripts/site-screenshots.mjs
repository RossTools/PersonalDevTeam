// The website's screenshots of PDT (PDT-273, PDT-280): the board, a workspace, the backlog, a workflow, a ticket, a
// branch's changes under review, a PM conversation, the Inbox, the Inbox on a phone, and Secrets, taken from a
// throwaway pdtd seeded with the made-up "Acme Shop" project (`PDT_SEED=website`, crates/pdtd/src/seed/website.rs),
// in the dark theme, and written as WebP to site/src/assets/screens/. Run it from the checkout after the builds:
//
//   cargo build -p pdtd && pnpm --dir web build
//   pnpm --dir web exec playwright-core install chromium   # once
//   node scripts/site-screenshots.mjs [--out <dir>]
//
// The daemon comes from scripts/scratch-daemon.sh (a free port, a scratch data folder, GitHub offline, in-memory
// secrets, no Tailscale), with a stand-in for every agent program: nothing real is started, and it never touches the
// stable PDT. The two Pipeline sessions it brings back (in their tickets' workspaces) run the stand-in, and their activity is reported as Claude's hooks
// would; the sample's dev server is the one real process (a static file server in the scratch folder).
import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// playwright-core is the web package's, sharp the site's (Astro's image service).
const { chromium } = createRequire(join(root, 'web/package.json'))('playwright-core');
const sharp = createRequire(join(root, 'site/package.json'))('sharp');

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const out = resolve(outIdx >= 0 ? args[outIdx + 1] : join(root, 'site/src/assets/screens'));
// The site's content column is ~1040px wide: a 1600×1000 window at device scale 2 stays sharp there and on large
// screens, and leaves the board's seven columns room for their titles.
const VIEWPORT = { width: 1600, height: 1000 };
const SCALE = 2;
// The phone shot: an iPhone's viewport, at its own device scale.
const devicesPhone = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true };

for (const [path, build] of [['target/debug/pdtd', 'cargo build -p pdtd'], ['web/dist/index.html', 'pnpm --dir web build']]) {
  if (!existsSync(join(root, path))) {
    console.error(`site-screenshots: no ${path}; build it first (${build})`);
    process.exit(1);
  }
}

// The stand-in for claude, codex and devin: a terminal that says it's working, and stays up for an hour at most.
const scratch = mkdtempSync(join(tmpdir(), 'pdt-site-shots-'));
const agent = join(scratch, 'agent.sh');
writeFileSync(agent, '#!/bin/sh\nprintf "Working on it…\\r\\n"\nexec sleep 3600\n');
chmodSync(agent, 0o755);

const daemon = join(root, 'scripts/scratch-daemon.sh');
const started = execFileSync(daemon, ['start', '--seed', 'website'], {
  cwd: root,
  encoding: 'utf8',
  env: { ...process.env, PDT_CLAUDE_PROGRAM: agent, PDT_CODEX_PROGRAM: agent, PDT_DEVIN_PROGRAM: agent },
  stdio: ['ignore', 'pipe', 'inherit'],
});
const port = /^port=(\d+)$/m.exec(started)?.[1];
const base = /^url=(.+)$/m.exec(started)?.[1];
if (!port || !base) {
  console.error(`site-screenshots: the scratch daemon did not say where it is:\n${started}`);
  process.exit(1);
}

async function api(path, init) {
  const res = await fetch(`${base}/api${path}`, {
    ...init,
    headers: { 'content-type': 'application/json', ...(init?.headers ?? {}) },
  });
  if (!res.ok) throw new Error(`${init?.method ?? 'GET'} ${path}: ${res.status} ${await res.text()}`);
  return res.status === 204 ? null : res.json();
}

/** Polls `check` until it returns something truthy (or throws after `ms`). */
async function until(what, check, ms = 15_000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await check();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 250));
  }
}

/** Brings the sample's sessions and runs to life: two Pipeline steps at work, the dev server up. */
async function stage() {
  const [project] = await api('/projects');
  const sessions = await api(`/projects/${project.id}/sessions`);
  const tickets = (await api(`/tickets?project=${project.id}&limit=100`)).items;
  const ticket = (title) => tickets.find((t) => t.title === title);
  // A Pipeline step's session on a ticket, by its persona.
  const step = (title, persona) => sessions.find((s) => s.persona?.name === persona && s.ticket?.id === ticket(title).id);
  const search = ticket('Search products by name');
  const cart = ticket('Save the cart across visits');
  // The Reviewer is working; QA waits on a permission prompt.
  for (const [s, event] of [
    [step(cart.title, 'Reviewer'), 'UserPromptSubmit'],
    [step('Guest checkout without an account', 'QA'), 'PermissionRequest'],
  ]) {
    await api(`/sessions/${s.id}/restart`, { method: 'POST', body: JSON.stringify({ cols: 120, rows: 30 }) });
    await until(`${s.title} to run`, async () => (await api(`/sessions/${s.id}`)).status === 'running');
    await api(`/sessions/${s.id}/activity`, { method: 'POST', body: JSON.stringify({ source: 'claude', event }) });
  }
  // The search branch's dev server: its tab opens in the ticket's workspace.
  const coder = step(search.title, 'Coder');
  const run = await api(`/projects/${project.id}/runs`, {
    method: 'POST',
    body: JSON.stringify({ worktreeId: coder.worktreeId, name: 'dev', kind: 'run' }),
  });
  await until('the dev server', async () => ['ready', 'running'].includes((await api(`/runs/${run.id}`)).state));
  // Each started ticket's workspace, as pickup made it.
  const workspaces = await api('/workspaces');
  const workspaceOf = (t) => workspaces.find((w) => w.ticket?.id === t.id);
  const searchWs = workspaceOf(search);
  const coderTab = searchWs.tabs.find((t) => t.sessionId === coder.id);
  // The Reviewer's branch, open for review with the user's line comment.
  const cartWs = workspaceOf(cart);
  const changesTab = cartWs.tabs.find((t) => t.worktreeId === step(cart.title, 'Reviewer').worktreeId);
  const guest = ticket('Guest checkout without an account');
  const pm = (await api('/pm/sessions'))[0].session;
  // Secrets for the Secrets page (the scratch daemon keeps them in memory): values nobody sees again.
  for (const [name, description] of [
    ['NPM_TOKEN', 'Publishes the shop\'s packages to npm.'],
    ['STRIPE_TEST_KEY', 'The payment provider\'s test key, for checkout tests.'],
    ['SMTP_PASSWORD', 'Sends the receipt emails from the staging mail server.'],
  ]) {
    await api('/secrets', { method: 'POST', body: JSON.stringify({ name, description, value: `sample-${name.toLowerCase()}-value` }) });
  }
  const workflow = (await api('/workflows')).find((w) => w.name === 'Nightly dependency updates');
  const detail = await api(`/workflows/${workflow.id}`);
  // Issues show on the graph's nodes.
  if (detail.issues?.length) throw new Error(`the sample workflow has issues: ${JSON.stringify(detail.issues)}`);
  return { project, searchWs, coderTab, cartWs, changesTab, guest, pm, workflow };
}

async function shoot(page, name, clip) {
  // The UI polls every 2 s (never network-idle): give its first answers and the fonts time to land.
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(2500);
  const png = await page.screenshot(clip ? { clip } : {});
  const file = join(out, `${name}.webp`);
  await sharp(png).webp({ quality: 82, effort: 6 }).toFile(file);
  console.log(file);
}

/** The graph's nodes with a margin, made 2:1 where the canvas allows (the site shows it full width). */
async function nodesClip(page) {
  const canvas = await page.getByLabel('Workflow graph').boundingBox();
  const boxes = await Promise.all((await page.locator('[data-testid^="flow-node-"]').all()).map((n) => n.boundingBox()));
  const pad = 48;
  let x0 = Math.min(...boxes.map((b) => b.x)) - pad;
  let x1 = Math.max(...boxes.map((b) => b.x + b.width)) + pad;
  let y0 = Math.min(...boxes.map((b) => b.y)) - pad;
  let y1 = Math.max(...boxes.map((b) => b.y + b.height)) + pad;
  const grow = (x1 - x0) / 2 - (y1 - y0);
  if (grow > 0) [y0, y1] = [y0 - grow / 2, y1 + grow / 2];
  x0 = Math.max(x0, canvas.x);
  y0 = Math.max(y0, canvas.y);
  x1 = Math.min(x1, canvas.x + canvas.width);
  y1 = Math.min(y1, canvas.y + canvas.height);
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

let browser;
try {
  const { project, searchWs, coderTab, cartWs, changesTab, guest, pm, workflow } = await stage();
  mkdirSync(out, { recursive: true });
  browser = await chromium.launch();
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: SCALE, colorScheme: 'dark' });
  await context.addInitScript((projectId) => {
    // No toasts over the screens (the sample's failed CI makes an Inbox notice).
    addEventListener('DOMContentLoaded', () => {
      const style = document.createElement('style');
      style.textContent = '[data-sonner-toaster] { display: none !important; }';
      document.head.append(style);
    });
    localStorage.setItem('pdt.theme', 'dark');
    localStorage.setItem('pdt.rightPanel.open', 'false');
    localStorage.setItem(
      'pdt.tickets.view',
      JSON.stringify({ projectId, mode: 'backlog', filters: { group: 'stage', sort: 'rank' } }),
    );
  }, project.id);
  // No DEV badge: the shots show PDT as people run it.
  await context.route('**/api/health', async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), dev: false } });
  });
  // Secrets as people keep them (the scratch daemon's are in memory): in the OS keychain.
  await context.route('**/api/secrets/store', (route) =>
    route.fulfill({ json: { backend: 'keychain', path: null, problem: null } }),
  );
  const page = await context.newPage();

  // The board's lanes end about 750px down: a shorter window leaves no empty band under them.
  await page.setViewportSize({ width: VIEWPORT.width, height: 800 });
  await page.goto(`${base}/tickets?view=board`);
  await page.getByText('Your turn').first().waitFor();
  await shoot(page, 'board');
  await page.setViewportSize(VIEWPORT);

  // The search ticket's workspace on its Coder's conversation, the Tickets section beside it.
  await page.goto(`${base}/sessions/${searchWs.id}/${coderTab.id}`);
  await page.getByText('Search is in.').waitFor();
  // The turn's tool calls, folded into one line once it ended.
  await page.getByText(/^Ran \d+ commands/).click();
  await shoot(page, 'workspaces');

  await page.goto(`${base}/tickets`);
  await page.getByText('Fix price rounding in cart totals').waitFor();
  await shoot(page, 'backlog');

  // A part of a parent ticket in QA, with its blockers, change request and sessions.
  await page.goto(`${base}/tickets/${guest.id}`);
  await page.getByText('An email address and a delivery address').first().waitFor();
  // The ticket's panel alone (the site shows it beside the board), once it has slid in.
  await page.waitForTimeout(1000);
  await shoot(page, 'ticket', await page.getByRole('dialog').boundingBox());

  await page.goto(`${base}/sessions/${cartWs.id}/${changesTab.id}`);
  await page.getByText('Cap the merged line').first().waitFor();
  await shoot(page, 'review');

  // The PM's answer and the Inbox take the top of the window: a shorter one leaves no empty band under them.
  await page.setViewportSize({ width: VIEWPORT.width, height: 600 });
  await page.goto(`${base}/pm/${pm.id}`);
  await page.getByText('Three things wait on you').waitFor();
  await shoot(page, 'pm');

  await page.goto(`${base}/inbox`);
  await page.getByText('attach a PDF invoice').first().waitFor();
  // The list and the open item, without the empty right of the window (the site shows it beside the phone's).
  await shoot(page, 'inbox', { x: 0, y: 0, width: 1100, height: 600 });

  await page.goto(`${base}/settings/secrets`);
  await page.getByText('STRIPE_TEST_KEY').first().waitFor();
  await shoot(page, 'secrets');

  await page.setViewportSize(VIEWPORT);

  // The phone's Inbox, at an iPhone's size.
  const phone = await browser.newContext({ ...devicesPhone, colorScheme: 'dark' });
  await phone.addInitScript(() => localStorage.setItem('pdt.theme', 'dark'));
  await phone.route('**/api/health', async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), dev: false } });
  });
  const phonePage = await phone.newPage();
  await phonePage.goto(`${base}/inbox`);
  await phonePage.getByText('attach a PDF invoice').first().waitFor();
  await shoot(phonePage, 'phone');
  await phone.close();

  // The graph alone, in a wider window so it fits at a size its node titles read at, cropped to its nodes.
  await page.setViewportSize({ width: 2000, height: 1000 });
  await page.goto(`${base}/workflows/${workflow.id}/graph`);
  await page.getByText('Ticket for the Coder').first().waitFor();
  await page.getByRole('button', { name: /fit/i }).first().click();
  await page.waitForTimeout(500);
  await shoot(page, 'workflows', await nodesClip(page));
} finally {
  await browser?.close();
  try {
    execFileSync(daemon, ['stop', port], { cwd: root, stdio: 'inherit' });
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}
