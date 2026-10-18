# Agent CLIs on a subscription: what the terms say (checked 2026-10-18)

Summaries of the vendors' published terms and docs, not legal advice. "Q:" is a verbatim quote. Anthropic rewrote
its Claude Code legal page twice in 2026: re-check every page here before any release to other people.

## Anthropic — `claude -p` stream-json on a Max login

**Personal use: allowed.** The Consumer Terms (§3.7, effective 2025-10-08, https://www.anthropic.com/legal/consumer-terms)
ban "automated or non-human means, whether through a bot, script, or otherwise", Q: "Except when you are accessing our
Services via an Anthropic API Key or where we otherwise explicitly permit it". Anthropic explicitly permits it:
- Q: "When you're signed in with your Claude plan instead, claude -p and Agent SDK usage still draw from your plan's
  usage limits" (https://support.claude.com/en/articles/17154008).
- Q: "You can still use the Claude Agent SDK, claude -p, and third-party apps with your subscription limits."
  (https://support.claude.com/en/articles/15036540, updated 2026-10-07).
- Limit: Q: "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the
  Agent SDK." (https://code.claude.com/docs/en/legal-and-compliance).

**Distribution (others install PDT): plausible as bring-your-own-CLI, not settled.** The legal page now says Q:
"Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route
requests through Free, Pro, or Max plan credentials on behalf of their users. Moreover, developers may not collect,
store, or intermediate Claude.ai credentials or session tokens" and also Q: "Nor does it prevent an end user from
signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts
Claude Code". Conditions: the binary unmodified, its login methods intact, each user's own credentials, no reselling,
no Anthropic names in the product name. Unclear: whether a local app spawning the user's installed `claude` counts as
"running Claude Code in your products" (that needs the Commercial Terms). The Agent SDK overview still says Q: "Unless
previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for
their products" (https://code.claude.com/docs/en/agent-sdk/overview). Safe route for a release: API keys or asking
Anthropic.

**Watch:** policy moved in 2026 (April: third-party tools to extra usage; May: a separate Agent SDK credit for `-p`,
paused in June, "For now, nothing has changed"), so `-p` usage could move to a separate pool. Q: "In bare mode, Claude
Code never reads OAuth credentials", and `--bare` "will become the default for -p in a future release"
(https://code.claude.com/docs/en/headless): PDT must notice that change. Max plans include monthly API credits
($100 / $200) usable with an API key from the claimed Console organization: a ready-made API-key option.

## OpenAI — `codex app-server` on a ChatGPT login

**Personal use: allowed.** Q: "If you've built a local or open-source application using Codex app-server
authentication, you can continue using it, though we recommend migrating to Sign in with ChatGPT so users have greater
control over and visibility into their usage. ... App-server authentication has never been permitted for commercial
or hosted services." (https://developers.openai.com/codex/app-server, "Auth endpoints").

**Distribution:** free, open-source, local: allowed. Paid or hosted: needs Sign in with ChatGPT and OpenAI's partner
form (https://developers.openai.com/siwc/token-sharing-open-source) — and then PDT would hold OAuth tokens itself,
against its "never handle credentials" rule. Unclear: whether remote access to the owner's own machine over Tailscale
is "hosted" (reading: no).

**Limits:** no circumventing rate limits; no sharing the account (Terms of Use, 2026-01-01). Codex auth docs Q: "Use
API key authentication for programmatic Codex CLI workflows, such as CI/CD jobs." — a recommendation, not a ban.

## Cognition — `devin acp` on a Devin Enterprise login

**Vendor side: allowed** (`devin acp` is Q: "intended to be invoked by an ACP-aware editor or IDE ... as a
subprocess"; no list of approved clients). Platform Terms §2.1 (2026-06-30): Q: "for your internal business purposes
only, solely for use by you and your Authorized Users". **The deciding rules are the employer's** enterprise agreement
and IT policy (including whether PDT may run on the work laptop): ask IT first. The AUP forbids sending credentials in
content and sharing the account.

## Comparable tools

Zed embeds Claude Code and Codex the same way: Q: "Claude Agent owns its own authentication and billing"
(https://zed.dev/docs/ai/external-agents).
