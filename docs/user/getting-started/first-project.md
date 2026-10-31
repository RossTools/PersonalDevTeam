# Add a first project

A project is a git repository PDT works in. Each ticket you start gets its own branch and folder (a git worktree) in
the project, so agents never edit your main checkout and several can work at once.

## Add it

In **Get started**, open **Add a project** and click **Choose a folder…** (or **Add project** in the
Settings list of projects). Then pick one:

- **Folder**: a repository already on your machine. Give the **Repository path**, and a **Name** if you want one other
  than the folder's.
- **Clone URL**: a repository to clone. Give the **Repository URL** and, under **Into folder**, where to clone it.
  With GitHub connected, **Clone from GitHub…** in **Get started** opens this directly.
- **Personal (no repo)**: for work that isn't code (notes, research, planning). Each ticket works in a folder of its
  own.

Each project has a **Key** of 2–10 letters and digits, suggested from its name, that numbers its tickets: `ALP-1`,
`ALP-2` and so on.

Clones go in your projects folder, `~/work` unless you change it in **Settings › This host › Projects folder**.

## What PDT uses from your repository

- **The base branch**: your repository's default branch, usually `main`. New tickets branch from it and are reviewed
  against it. Change it in the project's settings.
- **The `origin` remote**: when it's on GitHub (and GitHub is connected), PDT follows the project's pull requests and
  CI.

PDT needs no files of its own in your repository.

## Optional: how to run and test it

**Run** and **Test** start your project on a branch, each on its own port with its own data, as described in
`.pdt/project.toml` in your repository. To have an agent write it, open the project's settings and, under **Runs on
this host**, click **Set up with an agent**. Agents can work on tickets without it. See
[Run configuration](../automation/run-configuration.md).

## Optional: instructions for agents

**Agent instructions** in the project's settings is anything every agent on this project should know (how to run the
checks, conventions, what not to touch). PDT adds it to what it tells each agent it starts here. Agents still read
their usual files, such as `AGENTS.md` or `CLAUDE.md`.

## Next

[A first ticket](first-ticket.md).
