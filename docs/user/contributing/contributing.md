# Contributing

Report a bug or a security issue, suggest a feature, or work on PDT itself.

## Report a bug

Use **Report a bug…** in PDT (in the command palette, or **Settings › This host › Bug reports**). Fill in what
happened, what you expected and how to reproduce it. **Include diagnostics** attaches the end of PDT's log, with
tokens and your machine's names taken out; you can read it before you send. The dialog says where the report goes:
usually a public issue on GitHub, filed with your [update token](../your-machine/updates.md#the-github-token-for-updates).

You can also open an issue on [GitHub](https://github.com/RossTools/PersonalDevTeam/issues/new/choose).

## Report a security issue

Report vulnerabilities privately, never in a public issue: use
[GitHub's private reporting](https://github.com/RossTools/PersonalDevTeam/security/advisories/new) or email
[security@pdt.dev](mailto:security@pdt.dev). Include your PDT version, your OS, steps to reproduce and what an
attacker could do.

## Suggest a feature

Open a [feature request](https://github.com/RossTools/PersonalDevTeam/issues/new/choose) on GitHub: what you're
trying to do, what gets in the way, and how you picture it working.

## Work on PDT itself

PDT is built with PDT. Start with the developer guide:

- [How PDT is built](../../dev/architecture.md): the processes, the parts of the code and where each area lives.
- [Invariants that are easy to break](../../dev/invariants.md): the rules a change must keep.
- [Testing PDT](../../dev/testing.md): how the tests run and what they never touch.
- [How we work on PDT](../../SDLC.md): from a branch to a release.
