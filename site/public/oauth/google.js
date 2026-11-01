// Google's sign-in comes back here (https://pdt.dev/oauth/google), the only redirect URI of PDT's own OAuth clients,
// and this page sends the browser on to the PDT that started it: its address is in OAuth's `state`
// (`<nonce>.<base64url(origin)>`), which that PDT ties to its sign-in, with the PKCE verifier that makes the code
// useful. It forwards only to addresses that look like a PDT (the same rule as pdtd's `pdt_origin`), after asking,
// loads nothing from anywhere else and sends nothing anywhere else.

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

/** Where to send the browser for Google's answer (`?code&state`, or `?error&state`): `{ origin, url }`, or null. */
export function returnTo(search) {
  const q = new URLSearchParams(search);
  const state = q.get('state');
  const origin = stateOrigin(state);
  if (!origin) return null;
  const out = new URLSearchParams({ state });
  for (const k of ['code', 'error']) {
    const v = q.get(k);
    if (v !== null) out.set(k, v);
  }
  if (!out.has('code') && !out.has('error')) return null;
  return { origin, url: `${origin}/api/google/callback?${out}` };
}

function main() {
  const to = returnTo(window.location.search);
  // The code stays out of the address bar and history once read.
  window.history.replaceState(null, '', window.location.pathname);
  const $ = (id) => document.getElementById(id);
  if (!to) {
    $('refused').hidden = false;
    return;
  }
  $('where').textContent = to.origin;
  $('go').addEventListener('click', () => window.location.replace(to.url));
  $('ask').hidden = false;
}

if (typeof document !== 'undefined') main();
