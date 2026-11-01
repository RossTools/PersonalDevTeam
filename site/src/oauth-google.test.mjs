// The page Google's sign-in comes back to (public/oauth/google.js): it forwards only to addresses that look like a PDT
// and only states PDT could have issued. The same cases as pdtd's (crates/pdt-int-google/src/builtin.rs).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pdtOrigin, returnTo, stateOrigin } from '../public/oauth/google.js';

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
  const to = returnTo(`?state=${state}&code=4%2F0abc&scope=x`);
  assert.equal(to.origin, 'https://mac.tail1.ts.net');
  assert.equal(to.url, `https://mac.tail1.ts.net/api/google/callback?state=${state}&code=4%2F0abc`);
  assert.equal(returnTo(`?state=${state}&error=access_denied`).url, `https://mac.tail1.ts.net/api/google/callback?state=${state}&error=access_denied`);
  assert.equal(returnTo(`?state=${state}`), null);
  assert.equal(returnTo(`?code=x&state=${NONCE}.${b64('https://evil.example')}`), null);
  assert.equal(returnTo('?code=x'), null);
});
