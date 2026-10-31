# User docs: notes for writers

This folder is PDT's documentation for the people who use it. The website publishes it as its main docs: each
section is a folder, each page a Markdown file whose first `# heading` is its title. The sections, their pages and
their order are listed in `site/src/docs-pages.mjs` (`USER_SECTIONS`); a new page goes there too, or the site test
fails. This README is not published.

| Folder | Section |
| --- | --- |
| `getting-started/` | Getting started: install, first run, connecting agents and GitHub, a first project and ticket |
| `using-pdt/` | Using PDT: tickets and the board, workspaces and sessions, review and merge, the Inbox, documents |
| `automation/` | Automation: workflows, the PM, personas and approvals, plugins, run configuration, outside contributions |
| `your-machine/` | Your machine: install and updates, remote access and the phone, GitHub, secrets, settings, data |
| `troubleshooting/` | Troubleshooting & FAQ |
| `contributing/` | Contributing |

## Style

- **Second person, task-first.** Write to the reader ("you") about what they want to do: "Add a project", not "The
  Add project dialog". Lead with the steps; explain how it works afterwards, briefly, if at all.
- **Short sections.** One task per section, a few steps or paragraphs each. Prefer numbered steps for anything the
  reader does in order.
- **UI names in bold, exactly as PDT shows them**, including capitals and the ellipsis: **New ticket**, **Merge…**,
  **Settings › This host › Agents**. Check the app when unsure; don't paraphrase a button.
- **No internal references.** No ticket keys, code paths or source files, API routes, specs or plan phases: say what
  the reader sees and does. `pnpm --filter site check-docs` (and the site's tests in CI) fails on them; a legitimate
  exception goes in `ALLOW` in `site/src/user-docs-check.mjs`.
- **Link between pages** with relative links (`../using-pdt/review-and-merge.md#merge`); the site turns them into
  page links.
- **Screenshots** where they help a reader find something, not as decoration: take them with the docs screenshot
  script (PDT-273), on a scratch daemon with the demo data, so they match the current app and can be retaken.
- Plain Markdown, no front matter. Keep the stubs' one-line summary until the page is written.
