// The page Google's sign-in comes back to (public/oauth/google.js): it forwards only to addresses that look like a PDT,
// only states PDT could have issued, and only sign-ins that PDT at that address started there. The same address cases
// as pdtd's (crates/pdt-int-google/src/builtin.rs).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { STARTED_MS, loopback, pdtOrigin, remember, returnTo, startState, started, stateOrigin, take } from '../public/oauth/google.js';

const b64 = (s) => Buffer.from(s).toString('base64url');
const NONCE = 'abcdefghijklmnopqrstuvwxyz012-_A';

test('only addresses that look like a PDT', () => {
  for (const ok of [
    'http://127.0.0.1:7420', 'http://localhost:7421', 'http://[::1]:7420', 'http://192.168.1.20:7420', 'http://10.0.0.5:7420',
    'http://172.20.1.1:7420', 'http://100.101.102.103:7420', 'https://mac.tail1234.ts.net', 'http://mac.tail1234.ts.net:7420',
    'http://[fd7a:115c:a1e0::1]:7420',
  ]) assert.equal(pdtOrigin(ok), ok, ok);
  for (const bad of [
    'https://evil.example', 'https://ts.net', 'https://evil.com/.ts.net', 'https://evil.com#.ts.net', 'https://evil.com?.ts.net',
    'http://127.0.0.1.evil.com', 'http://user@127.0.0.1:7420', 'http://127.0.0.1:7420/api', 'http://8.8.8.8', 'http://172.32.0.1',
    'http://100.128.0.1', 'http://[2001:db8::1]', 'http://[fc::1]', 'http://127.0.0.1:99999', 'http://127.0.0.1:', 'javascript:alert(1)',
    'file:///etc/passwd', 'ftp://127.0.0.1', '', 'http://0x7f.0.0.1', 'http://2130706433', 'HTTP://127.0.0.1', 'https://mac.tail1.ts.net:443',
    null,
  ]) assert.equal(pdtOrigin(bad), null, String(bad));
});

test('a state names its PDT only when PDT could have issued it', () => {
  assert.equal(stateOrigin(`${NONCE}.${b64('http://127.0.0.1:7420')}`), 'http://127.0.0.1:7420');
  // pdtd's own encoding (builtin.rs `the_state_carries_the_origin`).
  assert.equal(stateOrigin(`${NONCE}.aHR0cDovLzEyNy4wLjAuMTo3NDIw`), 'http://127.0.0.1:7420');
  assert.equal(stateOrigin(`${NONCE}.${b64('https://evil.example')}`), null);
  assert.equal(stateOrigin(`short.${b64('http://127.0.0.1:7420')}`), null);
  assert.equal(stateOrigin(b64('http://127.0.0.1:7420')), null);
  assert.equal(stateOrigin(`${NONCE}.not base64!`), null);
  assert.equal(stateOrigin(null), null);
});

test('the browser goes back to its PDT with the code, or with what Google said', () => {
  const state = `${NONCE}.${b64('https://mac.tail1.ts.net')}`;
  const ours = () => true;
  const to = returnTo(`?state=${state}&code=4%2F0abc&scope=x`, ours);
  assert.equal(to.origin, 'https://mac.tail1.ts.net');
  assert.equal(to.url, `https://mac.tail1.ts.net/api/google/callback?state=${state}&code=4%2F0abc`);
  assert.equal(returnTo(`?state=${state}&error=access_denied`, ours).url, `https://mac.tail1.ts.net/api/google/callback?state=${state}&error=access_denied`);
  assert.equal(returnTo(`?state=${state}`, ours), null);
  assert.equal(returnTo(`?code=x&state=${NONCE}.${b64('https://evil.example')}`, ours), null);
  assert.equal(returnTo('?code=x', ours), null);
});

const SELF = 'https://pdt.dev/oauth/google';
const consent = (state, redirect = SELF, base = 'https://accounts.google.com/o/oauth2/v2/auth') =>
  `${base}?${new URLSearchParams({ client_id: 'c', redirect_uri: redirect, state, code_challenge: 'x' })}`;

test('a sign-in starts here only from the PDT its state names', () => {
  const state = `${NONCE}.${b64('https://mac.tail1.ts.net')}`;
  assert.equal(startState(consent(state), 'https://mac.tail1.ts.net', SELF), state);
  // Another address (an attacker's own *.ts.net, sending the victim's PDT's state, or the reverse) can't.
  assert.equal(startState(consent(state), 'https://evil.tail9.ts.net', SELF), null);
  assert.equal(startState(consent(`${NONCE}.${b64('https://evil.example')}`), 'https://evil.example', SELF), null);
  // Only to Google's consent page, coming back here.
  assert.equal(startState(consent(state, 'https://evil.example/cb'), 'https://mac.tail1.ts.net', SELF), null);
  assert.equal(startState(consent(state, SELF, 'https://evil.example/o/oauth2/v2/auth'), 'https://mac.tail1.ts.net', SELF), null);
  assert.equal(startState('not a url', 'https://mac.tail1.ts.net', SELF), null);
  assert.equal(startState(undefined, 'https://mac.tail1.ts.net', SELF), null);
});

function memory() {
  const m = new Map();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, String(v)) };
}

test('a code goes back only for a sign-in started here, once and in time', () => {
  const s = memory();
  const state = `${NONCE}.${b64('http://127.0.0.1:7420')}`;
  const other = `${'z'.repeat(32)}.${b64('https://evil.tail9.ts.net')}`;
  remember(s, `old.${b64('x')}`, 1000 - STARTED_MS);
  remember(s, state, 1000);
  assert.deepEqual(Object.keys(started(s, 1000)), [state]);
  const ours = (st) => take(s, st, 2000);
  // Someone else's Google link: its state never started here.
  assert.equal(returnTo(`?state=${other}&code=c`, ours), null);
  assert.equal(returnTo(`?state=${state}&code=c`, ours).origin, 'http://127.0.0.1:7420');
  assert.equal(returnTo(`?state=${state}&code=c`, ours), null);
  remember(s, state, 0);
  assert.equal(take(s, state, STARTED_MS), false);
  s.setItem('pdt-google-sign-ins', 'not json');
  assert.deepEqual(started(s, 0), {});
});

test('only this computer’s own addresses go back without a warning', () => {
  for (const o of ['http://127.0.0.1:7420', 'http://localhost:7421', 'http://pdt.localhost', 'http://[::1]:7420']) assert.ok(loopback(o), o);
  for (const o of ['https://mac.tail1.ts.net', 'http://192.168.1.20:7420', 'http://100.101.102.103:7420']) assert.ok(!loopback(o), o);
});
