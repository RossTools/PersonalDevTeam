# Security policy

Thanks for helping keep PDT and the people who use it safe. Please report vulnerabilities privately, never in a public
issue, pull request or discussion.

## Reporting a vulnerability

- **Preferred:** GitHub's private vulnerability reporting. Open the repo's **Security** tab and choose **Report a
  vulnerability** ([direct link](https://github.com/RossTools/PersonalDevTeam/security/advisories/new)). The report
  stays private between you and the maintainers, and the fix and advisory can be worked on there.
- **Or email** [security@pdt.dev](mailto:security@pdt.dev).

Please include:

- the PDT version (Settings → This host, or `GET /api/health`) and how it was installed;
- your OS and version;
- steps to reproduce, or a proof of concept;
- the impact: what an attacker can do, and what they need first (network access, a Tailscale login, a crafted issue
  or repo, ...).

## What to expect

PDT is a small project maintained in spare time, so these are aims rather than promises:

- an acknowledgement within about five working days;
- an assessment and, when the report is confirmed, a plan for a fix, with updates as it goes;
- a fixed release, followed by a GitHub security advisory published together with you (coordinated disclosure).
  Please give us a reasonable time to ship the fix before disclosing it yourself;
- credit in the advisory and the changelog, if you want it.

## Supported versions

Only the latest release on the stable channel gets security fixes. Older releases don't: PDT updates itself, so the
fix is to update to the latest release.

## Scope

In scope:

- `pdtd` (the daemon) and `pdt-host` (the supervisor and its updates);
- the web UI and the HTTP, WebSocket and MCP API;
- remote access (the `tailscale serve` mapping and its owner-only Host guard);
- the agent sandboxing and the prompt-injection protections for outside input (issues, bug reports, pull requests
  PDT didn't open);
- secrets handling: the keychain store, redaction of stored text and logs, and how agents use secrets without seeing
  them;
- the desktop tray app and the release bundles.

Out of scope:

- third-party agent CLIs (Claude Code, Codex, Devin and ACP agents) and the models behind them: report those to their
  vendors. A way to make PDT itself run one with more access than it should is in scope;
- the user's own machine setup: a compromised machine or account, agent settings or permissions the user chose, other
  software on the host, or Tailscale itself;
- findings that need an attacker who already has the owner's local access or Tailscale login.
