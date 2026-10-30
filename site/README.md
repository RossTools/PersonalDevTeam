# PDT website

The project website: a landing page and the docs, built with [Astro](https://astro.build) and
[Starlight](https://starlight.astro.build) and served by GitHub Pages (private for now: only members of the RossTools
org can open it).

- `src/pages/index.astro` is the landing page (its own page, outside Starlight).
- `src/pages/docs/index.astro` is the docs home: a card per section of the user docs. The docs pages themselves are
  the repository's Markdown, read in place from `../docs`: the user docs in `user/` (the main docs, at
  `/docs/<section>/<page>/`; their sections and pages, in sidebar order, are `USER_SECTIONS` in `src/docs-pages.mjs`,
  and `docs/user/README.md` has the style notes for writing them), plus `agent-guide.md` and `run-config.md` (under Guides), and the
  developer guide in `dev/` with `SDLC.md` (under Contributing, after its own page: `DEVELOPER_PAGES`). `superpowers/`, `prompts/`,
  `ROADMAP.md` and the READMEs in `user/` stay out. There is no copy step: edit the file in `docs/` and the next
  deploy shows it. A page's title is its first `# heading`; relative links between published docs point at their
  pages on the site, and links to anything else in the repository at the file on GitHub
  (`src/remark-docs-links.mjs`).
- The user docs are for people who use PDT, so `src/user-docs-check.mjs` fails on internal references in them
  (ticket keys, code paths and source files, crate names, API routes, specs, plan phases), with an allowlist for
  legitimate exceptions. The site's tests run it, so the `site` CI job does; `pnpm --filter site check-docs` runs it
  alone.
- The theme is the app's: dark first (a first visit starts dark whatever the system prefers; the theme menu can switch
  it), `#0e0f12` / `#17181d` / `#e7e8ec` with the `#3561b8` accent, Inter and JetBrains Mono served with the site
  (`src/styles/`). The logo files in `src/assets/` and `public/` are the logo set, unchanged.
- The "PDT is built by PDT" numbers and the version on the landing page are counted from git at build time
  (`src/stats.mjs`; a shallow clone falls back to fixed numbers).

## Screenshots

The landing page's screenshots of PDT (`src/assets/screens/`: the board, a workspace, the backlog and a workflow) are
made by a script, never edited by hand. From the repository root (macOS or Linux), after the builds:

```sh
cargo build -p pdtd && pnpm --dir web build
pnpm --dir web exec playwright-core install chromium   # once
node scripts/site-screenshots.mjs
```

It starts a throwaway pdtd (`scripts/scratch-daemon.sh`, so never the PDT you use) seeded with a made-up sample
project, Acme Shop (`PDT_SEED=website`, crates/pdtd/src/seed/website.rs), with GitHub offline, secrets in memory and
a stand-in for every agent program; takes each screen in the dark theme at 1600 px wide and device scale 2; writes
them as WebP; and stops the daemon. Astro's `<Image>` makes the sizes the page serves. Regenerate them when the UI
changes (the release checklist in docs/SDLC.md says when); to change what they show, change the seed.

## Local development

From the repository root, after `pnpm install`:

```sh
pnpm --filter site dev      # http://localhost:4321, reloads on edits to site/ and docs/
pnpm --filter site build    # into site/dist
pnpm --filter site preview  # serve site/dist
pnpm --filter site test     # the link-rewriting tests and the user docs check
pnpm --filter site check-docs  # the user docs check alone
```

## Deploys

`.github/workflows/pages.yml` builds the site and deploys it with `actions/deploy-pages` on every push to `main`
that touches `site/**` or `docs/**`, and when run by hand (Actions → pages → Run workflow). It takes the site's URL
and base path from `actions/configure-pages`, so the same build works on the private Pages domain, a project path or
a custom domain.

On pull requests and in the merge queue, `ci.yml`'s `site` job builds it whenever `site/**` or `docs/**` (or the pnpm
lockfile) changes. A pull request that only changes `site/`, `docs/` or Markdown runs only `changes` and `site`
(docs/superpowers/notes/ci.md).

Before the first deploy, in the repository's Settings → Pages: set Source to **GitHub Actions** and visibility to
**Private**.

## Going public

1. **Visibility.** Settings → Pages → visibility: **Public** (a public site needs the repository to be public, or an
   Enterprise plan for a public site from a private repository).
2. **Custom domain.** Settings → Pages → Custom domain: `pdt.dev`, then Save. With a workflow deploy no `CNAME` file is
   needed; `configure-pages` reports the domain, and the next deploy builds for `https://pdt.dev/`.
3. **DNS** at the registrar for `pdt.dev`:

   | Name  | Type  | Value                  |
   | ----- | ----- | ---------------------- |
   | `@`   | A     | `185.199.108.153`      |
   | `@`   | A     | `185.199.109.153`      |
   | `@`   | A     | `185.199.110.153`      |
   | `@`   | A     | `185.199.111.153`      |
   | `@`   | AAAA  | `2606:50c0:8000::153`  |
   | `@`   | AAAA  | `2606:50c0:8001::153`  |
   | `@`   | AAAA  | `2606:50c0:8002::153`  |
   | `@`   | AAAA  | `2606:50c0:8003::153`  |
   | `www` | CNAME | `rosstools.github.io`  |

4. **Verify the domain** for the RossTools org (org Settings → Pages → Add a domain): GitHub gives a TXT record
   (`_github-pages-challenge-rosstools.pdt.dev`) to add, then Verify. This stops anyone else's repository from
   claiming `pdt.dev`.
5. **Enforce HTTPS.** Once the certificate is issued (it can take up to a day after DNS resolves), tick Settings →
   Pages → Enforce HTTPS.
