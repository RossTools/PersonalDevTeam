# PDT website

The project website: a landing page and the docs, built with [Astro](https://astro.build) and
[Starlight](https://starlight.astro.build) and served by GitHub Pages (private for now: only members of the RossTools
org can open it).

- `src/pages/index.astro` is the landing page (its hero's screenshots are a carousel, `src/components/ShotCarousel.astro`:
  no library, no autoplay, and without JS the slides scroll sideways) and `src/pages/features.astro` the Features page (`/features/`: a
  section per area of PDT, each with its screenshots and a link to its user docs; the landing page's feature cards link
  to its sections). Both stand outside Starlight, in `src/layouts/Landing.astro` (head, nav, footer) with the styles in
  `src/styles/landing.css` and their links in `src/links.mjs`. The Features page's wording follows the user docs:
  claim only what ships, and change it when they change.
- `src/pages/docs/index.astro` is the docs home: a card per section of the user docs. The docs pages themselves are
  the repository's Markdown, read in place from `../docs`: the user docs in `user/` (the main docs, at
  `/docs/<section>/<page>/`, or `/docs/<section>/` for a page named after its section; their sections and pages, in sidebar order, are `USER_SECTIONS` in `src/docs-pages.mjs`,
  and `docs/user/README.md` has the style notes for writing them), plus `agent-guide.md` (under Guides), and the
  developer guide in `dev/` with `SDLC.md` (under Contributing, after its own page: `DEVELOPER_PAGES`). `prompts/`,
  `run-config.md` (the agents' part of the run-config guide) and the READMEs in `user/` stay out. There is no copy step: edit the file in `docs/` and the next
  deploy shows it. A page's title is its first `# heading`; relative links between published docs point at their
  pages on the site, and links to anything else in the repository at the file on GitHub
  (`src/remark-docs-links.mjs`). Its meta description (search results, link previews) is its first paragraph
  (`src/doc-description.mjs`), so that paragraph should say what the page is for. A page that moves gets a redirect
  from its old address (`MOVED` in `src/docs-pages.mjs`).
- The user docs are for people who use PDT, so `src/user-docs-check.mjs` fails on internal references in them
  (ticket keys, code paths and source files, crate names, API routes, specs, plan phases), with an allowlist for
  legitimate exceptions, and on any file under `docs/` in a `superpowers`, `specs` or `plans` folder. The site's tests run it, so the `site` CI job does; `pnpm --filter site check-docs` runs it
  alone.
- The theme is the app's: dark first (a first visit starts dark whatever the system prefers; the theme menu can switch
  it), `#0e0f12` / `#17181d` / `#e7e8ec` with the `#3561b8` accent, Inter and JetBrains Mono served with the site
  (`src/styles/`). The logo files in `src/assets/` and `public/` are the logo set, unchanged.
- `src/pages/plugins/` are the Plugins pages, outside Starlight in the landing page's frame
  (`src/layouts/Landing.astro`): `/plugins/` lists every plugin by category, as the app's Settings › Plugins does,
  but the media ones, which are on `/plugins/media/`; each plugin has its page, `/plugins/<id>/`. They read
  `src/data/plugins.json`, which pdtd's integrations tests write from the same code the app lists its plugins from
  (`cargo test -p pdtd`): never edit it by hand. CI's check that generated files are committed fails when it is
  stale. "Request a plugin" opens the `plugin.yml` issue template.
- Every page, docs or landing frame, shares the head tags in `src/head.mjs`: `favicon.ico`, `apple-touch-icon.png` and
  the link preview image `og.png` (1200×630), all in `public/` and made by `node scripts/site-icons.mjs` (the preview
  is the landing hero drawn with the site's Inter; run it again when the logo or the hero's wording changes). Titles
  read "Page · PDT (Personal Dev Team)" everywhere (`pageTitle` in `src/brand.mjs`, Starlight's `titleDelimiter`).
- `src/pages/404.astro` is the page not found, in the landing frame and left out of search engines (`noindex`, no
  canonical address); Starlight's own 404 is off.
- `src/pages/privacy.astro` is the privacy page (`/privacy/`, linked from the footer and Google's consent screen).
  `public/oauth/google.html` (`/oauth/google`) is where Google's sign-in comes back to with PDT's own OAuth clients:
  `public/oauth/google.js` reads the PDT to return to from OAuth's `state`, forwards only to an address that looks
  like a PDT (the same rule as pdtd's, `crates/pdt-int-google/src/builtin.rs`) after asking, and loads and sends
  nothing else (a strict Content-Security-Policy, no referrer). `src/oauth-google.test.mjs` tests its rules. Keep it a
  static file: its address is the OAuth clients' registered redirect URI.
- `public/install.sh` (macOS and Linux) and `public/install.ps1` (Windows) are the one-line install scripts, served
  from the site's root (`curl -fsSL https://pdt.dev/install.sh | sh`, `irm https://pdt.dev/install.ps1 | iex`). They
  pick the release from GitHub's API, check it against its `SHA256SUMS` and install it as the user docs' install page
  says by hand. `src/install.test.mjs` runs install.sh against a fake GitHub API with stubbed `uname`, `pdt-host` and
  `loginctl` in a temp HOME; CI's `site` job also runs shellcheck and PSScriptAnalyzer on them. Their names (product,
  repository, port) are brand names: a pdt-core test checks them. `src/components/InstallBox.astro` shows the command
  (landing and Features pages): a tab per system, the visitor's picked by `src/install.mjs`, with a copy button, and
  the page's "Download for …" buttons follow the visitor's system.
- The "PDT is built by PDT" numbers and the version on the landing page are counted from git at build time
  (`src/stats.mjs`; a shallow clone falls back to fixed numbers).

## Screenshots

The screenshots of PDT on the landing and Features pages (`src/assets/screens/`: the board, a workspace, the backlog, a
workflow, a ticket, a branch's changes under review, two PM conversations, the Inbox, the Inbox on a phone, and Secrets)
are made by a script, never edited by hand. From the repository root (macOS or Linux), after the builds:

```sh
cargo build -p pdtd && pnpm --dir web build
pnpm --dir web exec playwright-core install chromium   # once
node scripts/site-screenshots.mjs
```

It starts a throwaway pdtd (`scripts/scratch-daemon.sh`, so never the PDT you use) seeded with a made-up sample
project, Acme Shop (`PDT_SEED=website`, crates/pdtd/src/seed/website.rs), with GitHub offline, secrets in memory and
a stand-in for every agent program; takes each screen in the dark theme at 1600 px wide and device scale 2 (the Inbox's at 1100 px, Secrets cropped to its
list so it reads on a phone, the phone's at 390 px and scale 3); writes
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
that touches `site/**` or `docs/**`, on the 1st of every month (so `/.well-known/security.txt`, whose `Expires` is set
at build time to 180 days out, never runs out), and when run by hand (Actions → pages → Run workflow). It takes the site's URL
and base path from `actions/configure-pages`, so the same build works on the private Pages domain, a project path or
a custom domain.

On pull requests and in the merge queue, `ci.yml`'s `site` job builds it whenever `site/**` or `docs/**` (or the pnpm
lockfile) changes. A pull request that only changes `site/`, `docs/` or Markdown runs only `changes` and `site`
(docs/SDLC.md "CI").

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
   | `@`   | TXT   | `v=spf1 include:_spf.google.com ~all` |
   | `_dmarc` | TXT | `v=DMARC1; p=quarantine; rua=mailto:security@pdt.dev` |

   The SPF and DMARC records are for the domain's mail (`security@pdt.dev` in SECURITY.md and `security.txt`): without
   them anyone can send mail that claims to come from it.

4. **Verify the domain** for the RossTools org (org Settings → Pages → Add a domain): GitHub gives a TXT record
   (`_github-pages-challenge-rosstools.pdt.dev`) to add, then Verify. This stops anyone else's repository from
   claiming `pdt.dev`.
5. **Enforce HTTPS.** Once the certificate is issued (it can take up to a day after DNS resolves), tick Settings →
   Pages → Enforce HTTPS.
6. **Private vulnerability reporting.** Settings → Code security → Private vulnerability reporting: **Enable**.
   SECURITY.md, the issue chooser and `security.txt` all send reporters to its form first.
