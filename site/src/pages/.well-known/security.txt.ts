// security.txt (RFC 9116), for whatever URL the site is built for (src/security-txt.mjs).
import type { APIRoute } from 'astro';
import { securityTxt } from '../../security-txt.mjs';

export const GET: APIRoute = ({ site }) =>
  new Response(securityTxt(new URL('.well-known/security.txt', site).href), {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
