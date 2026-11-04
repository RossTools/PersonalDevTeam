// Google's sign-in comes back here (https://pdt.dev/oauth/google), the only redirect URI of PDT's own OAuth clients,
// and this page sends the browser on to the PDT that started it: its address is in OAuth's `state`
// (`<nonce>.<base64url(origin)>`), which that PDT ties to its sign-in, with the PKCE verifier that makes the code
// useful. It forwards only to addresses that look like a PDT (the same rule as pdtd's `pdt_origin`), after asking,
// loads nothing from anywhere else and sends nothing anywhere else.
//
// It also starts each sign-in, so a code goes back only to the PDT that started it here, in this browser (PDT-410):
// opened with no query by PDT's sign-in window, it says it is ready to its opener, takes Google's consent address in
// a message whose origin the browser vouches for, remembers the state when its origin is that one, and goes on to
// Google. A state it doesn't remember (a Google link someone else made) goes nowhere.
//
// Where it can't hand the answer back (a corporate proxy isolating this page in a remote browser: no opener, no
// storage of its own, and 127.0.0.1 there isn't the user's), it shows Google's answer as a code to paste into PDT
// (PDT-590). A code is useless without the sign-in's state and PKCE verifier, which only the PDT that started it
// has, and it goes nowhere by itself.

const NONCE = /^[A-Za-z0-9_-]{32}$/;

function ipv4(host) {
  const parts = host.split('.');
  if (parts.length !== 4 || !parts.every((p) => /^\d{1,3}$/.test(p) && Number(p) <= 255)) return null;
  return parts.map(Number);
}

/** `origin`, when it looks like a PDT: loopback, a private or Tailscale address, `localhost` or `*.ts.net`. */
export function pdtOrigin(origin) {
  if (typeof origin !== 'string' || !/^https?:\/\/[^/?#@\\\s%]+$/i.test(origin)) return null;
  let u;
  try {
    u = new URL(origin);
  } catch {
    return null;
  }
  // Exactly as browsers write it (pdtd puts it in the state so): no user, path, capitals or odd number forms.
  if (u.origin !== origin) return null;
  const host = u.hostname;
  let ok = false;
  if (host.startsWith('[')) {
    const ip = host.slice(1, -1);
    ok = ip === '::1' || /^f[cd][0-9a-f]{2}:/.test(ip);
  } else if (ipv4(host)) {
    const [a, b] = ipv4(host);
    ok = a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127);
  } else {
    const labels = host.split('.');
    const named = labels.every((l) => /^[a-z0-9-]+$/.test(l));
    ok = named && (host === 'localhost' || host.endsWith('.localhost') || (labels.length > 2 && host.endsWith('.ts.net')));
  }
  return ok ? u.origin : null;
}

/** Whether `origin` (one `pdtOrigin` took) is this machine's own, not one on the network or Tailscale. */
export function loopback(origin) {
  const host = new URL(origin).hostname;
  return host === 'localhost' || host.endsWith('.localhost') || host === '[::1]' || /^127\./.test(host);
}

function fromBase64Url(s) {
  if (!/^[A-Za-z0-9_-]+$/.test(s)) return null;
  try {
    return atob(s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (s.length % 4)) % 4));
  } catch {
    return null;
  }
}

/** The PDT a `state` names, when it is one PDT could have issued. */
export function stateOrigin(state) {
  if (typeof state !== 'string') return null;
  const dot = state.indexOf('.');
  if (dot < 0 || !NONCE.test(state.slice(0, dot))) return null;
  return pdtOrigin(fromBase64Url(state.slice(dot + 1)));
}

const GOOGLE = 'https://accounts.google.com/o/oauth2/v2/auth';
/** How long a started sign-in is remembered: as long as pdtd keeps it. */
export const STARTED_MS = 15 * 60 * 1000;
const KEY = 'pdt-google-sign-ins';

/**
 * The state of a sign-in PDT at `from` (a message's origin, as the browser gives it) may start here: `url` is
 * Google's consent page coming back to `self` (this page's address), for a state naming `from`. Otherwise null.
 */
export function startState(url, from, self) {
  if (typeof url !== 'string' || typeof from !== 'string') return null;
  let u;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (`${u.origin}${u.pathname}` !== GOOGLE || u.searchParams.get('redirect_uri') !== self) return null;
  const state = u.searchParams.get('state');
  return stateOrigin(state) === from ? state : null;
}

/** The sign-ins started here, `{ state: startedMs }`, with those over `STARTED_MS` old left out. */
export function started(store, now) {
  let all;
  try {
    all = JSON.parse(store.getItem(KEY) ?? '{}');
  } catch {
    all = {};
  }
  const out = {};
  for (const [state, at] of Object.entries(all && typeof all === 'object' ? all : {})) {
    if (typeof at === 'number' && now - at >= 0 && now - at < STARTED_MS) out[state] = at;
  }
  return out;
}

/** Remembers that the sign-in `state` started here. */
export function remember(store, state, now) {
  store.setItem(KEY, JSON.stringify({ ...started(store, now), [state]: now }));
}

/** Whether the sign-in `state` started here (in time), forgetting it: each comes back once. */
export function take(store, state, now) {
  const all = started(store, now);
  const ok = typeof state === 'string' && Object.hasOwn(all, state);
  delete all[state];
  store.setItem(KEY, JSON.stringify(all));
  return ok;
}

/**
 * Where to send the browser for Google's answer (`?code&state`, or `?error&state`): `{ origin, url }`, or null. Only
 * for a sign-in that started here (`ours(state)`).
 */
export function returnTo(search, ours) {
  const q = new URLSearchParams(search);
  const state = q.get('state');
  const origin = stateOrigin(state);
  if (!origin || !ours(state)) return null;
  const out = new URLSearchParams({ state });
  for (const k of ['code', 'error']) {
    const v = q.get(k);
    if (v !== null) out.set(k, v);
  }
  if (!out.has('code') && !out.has('error')) return null;
  return { origin, url: `${origin}/api/google/callback?${out}` };
}

/** Google's answer (`state`, `code`) as one line to paste into PDT: base64url of `state=…&code=…`. */
export function pasteCode(state, code) {
  const q = new URLSearchParams({ state, code }).toString();
  return btoa(q).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/**
 * What to show for pasting into PDT, for Google's answer (`?code&state`) to a sign-in a PDT could have started:
 * `{ origin, code }`, or null.
 */
export function pasteFor(search) {
  const q = new URLSearchParams(search);
  const state = q.get('state');
  const code = q.get('code');
  const origin = stateOrigin(state);
  return origin && code ? { origin, code: pasteCode(state, code) } : null;
}

function store() {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** Opened by PDT's sign-in window: wait for Google's address from the PDT that opened it, then go there. */
function begin($) {
  const opener = window.opener;
  const s = store();
  // PDT sends the window to Google itself when this page doesn't answer; the code then comes back to paste.
  $('starting').hidden = false;
  if (!opener || !s) return;
  // The redirect URI PDT registered, whatever path the host served this page at.
  const self = `${window.location.origin}/oauth/google`;
  window.addEventListener('message', (e) => {
    if (e.source !== opener || typeof e.data !== 'object' || e.data === null) return;
    const state = startState(e.data.pdtRelayTo, e.origin, self);
    if (!state) return;
    remember(s, state, Date.now());
    window.location.replace(e.data.pdtRelayTo);
  });
  // Nothing secret: it says only that this page is listening.
  opener.postMessage({ pdtRelay: 'ready' }, '*');
}

/** Shows the code to paste into PDT at `origin`. */
function showPaste($, paste) {
  $('code').value = paste.code;
  $('code-shown').value = paste.code;
  $('code-for').textContent = paste.origin;
  $('copy').addEventListener('click', () => {
    $('code-shown').select();
    navigator.clipboard?.writeText(paste.code).then(() => { $('copy').textContent = 'Copied'; }, () => {});
  });
}

function main() {
  const $ = (id) => document.getElementById(id);
  if (!window.location.search) {
    begin($);
    return;
  }
  const s = store();
  const search = window.location.search;
  const to = returnTo(search, (state) => s !== null && take(s, state, Date.now()));
  const paste = pasteFor(search);
  // The code stays out of the address bar and history once read.
  window.history.replaceState(null, '', window.location.pathname);
  if (paste) showPaste($, paste);
  if (!to) {
    // Not started here (or this browser kept no record of it): never sent on, only shown to paste.
    $(paste ? 'paste' : 'refused').hidden = false;
    return;
  }
  $('paste-instead').hidden = !paste;
  $('where').textContent = to.origin;
  $('remote').hidden = loopback(to.origin);
  $('go').addEventListener('click', () => window.location.replace(to.url));
  $('ask').hidden = false;
}

if (typeof document !== 'undefined') main();
