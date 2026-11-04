import { enc, request } from './core';
import type { SignInAccount } from './gen/SignInAccount';
import type { AccountSignInStart } from './gen/AccountSignInStart';
import type { AccountSignInDone } from './gen/AccountSignInDone';

/**
 * A plugin's account sign-in (PDT-378), under the routes its declaration names (`SignIn.api`): its accounts, adding
 * one with PDT's own client, signing one in (or letting agents use it) and out. Only PDT's web app signs in.
 */
export const signInApi = {
  /** The accounts at `url` (`{api}/accounts`, or a picker's `accounts`). */
  signInAccounts: (url: string) => request<SignInAccount[]>(url),
  /** Where the browser signs in to add an account, with PDT's own client; the service sends it back to `origin`. */
  signInAdd: (base: string, origin: string) =>
    request<AccountSignInStart>(`${base}/sign-in`, { method: 'POST', body: JSON.stringify({ origin }) }, { asUser: true }),
  /** Where the browser signs an account in (or, with `agent`, lets agents use it); it comes back to `origin`. */
  signInAccount: (base: string, connectionId: string, origin: string, agent = false) =>
    request<AccountSignInStart>(
      `${base}/accounts/${enc(connectionId)}/sign-in`,
      { method: 'POST', body: JSON.stringify(agent ? { origin, agent } : { origin }) },
      { asUser: true },
    ),
  /** Finishes a sign-in with the code the website's page showed, when it couldn't send the browser back. */
  signInPaste: (base: string, code: string) =>
    request<AccountSignInDone>(`${base}/sign-in/code`, { method: 'POST', body: JSON.stringify({ code }) }, { asUser: true }),
  signOutAccount: (base: string, connectionId: string) =>
    request<void>(`${base}/accounts/${enc(connectionId)}/sign-in`, { method: 'DELETE' }, { asUser: true }),
  /** Agents may no longer use the account. */
  forgetAgents: (base: string, connectionId: string) =>
    request<void>(`${base}/accounts/${enc(connectionId)}/agents`, { method: 'DELETE' }, { asUser: true }),
  /** A GET as PDT's web app, for a provider's picker module (its own routes). */
  pickerRequest: (path: string) => request<unknown>(path, undefined, { asUser: true }),
};
