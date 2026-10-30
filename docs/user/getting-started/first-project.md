# Add a first project

A project is a git repository PDT works in. Each ticket you start gets its own branch and its own folder (a git
worktree) inside the project, so agents never edit your main checkout and several can work at once.

## Add it

In **Get started**, open **Add a project** and click **Choose a folder…** (or **Add a project** in the **Sessions**
sidebar). The **Add project** dialog has three ways in:

- **Folder**: a repository already on your machine. Give the **Repository path** (and a **Name**, if you want one
  other than the folder's).
- **Clone URL**: a repository to clone. Give the **Repository URL** and, under **Into folder**, where to clone it
  (your projects folder unless you say otherwise). When GitHub is connected, **Clone from GitHub…** in **Get started** opens this
  directly.
- **Personal (no repo)**: a project with no repository, for work that isn't code (notes, research, planning). Each
  ticket works in a folder of its own.

Every project has a **Key** of 2–10 letters and digits, suggested from its name. Its tickets are numbered with it:
`ALP-1`, `ALP-2` and so on.

Your projects folder (where clones go) is `~/work` unless you change it in **Settings › This host › Projects folder**.

## What PDT uses from your repository

- **The base branch.** New tickets branch from it and are reviewed against it: your repository's default branch,
  usually `main`. Change it in the project's settings.
- **The `origin` remote.** When it's on GitHub (and GitHub is connected), PDT follows pull requests and CI for the
  project.

PDT doesn't need any files of its own in your repository.

## Optional: tell PDT how to run and test it

**Run** and **Test** start your project on a branch, each on its own port with its own data. PDT reads how to do that
from `.pdt/project.toml` in your repository. The quickest way to write one is to let an agent do it: in the project's
settings, under **Runs on this host**, click **Set up with an agent**.

You can skip this for now: agents can work on tickets without it.

## Optional: instructions for agents

The project's settings also hold **Agent instructions**: anything every agent working on this project should know
(how to run the checks, conventions, what not to touch). PDT adds them to what it tells each agent it starts here.
Agents still read their usual files in your repository, such as `AGENTS.md` or `CLAUDE.md`.

## Next

[A first ticket](first-ticket.md).
