# Remote projects

Let another instance of PDT work on one of your projects, with the board, tickets and PM staying where they are.

Each PDT you install is an **instance** with a name, like `studio` or `win-box`. A project belongs to one instance,
its **owner**, which keeps its board, tickets, workflows, Inbox and PM. Other instances can join it as **remote
instances** to work on it for the owner, for example to build and test on another operating system. Each instance
keeps its own projects as usual.

For now a remote instance joins the project and reports on itself; work isn't sent to it yet.

## Before you start

- **Name each instance** in **Settings › This host › Name** (or in **Get started**), so you can tell them apart.
- **Turn on remote access on both**, with the same Tailscale owner. See
  [Remote access and your phone](remote-access.md). The two instances reach each other through it.

## Add a remote instance to a project

1. **On the owner**, open the project's settings, then **Remote instances**, and click **Add**. PDT shows a pairing
   code and this instance's address. The code works once, for 10 minutes.
2. **On the other instance**, click **Add project**, choose **Remote project**, and enter the owner's address and the
   code. Click **Join project**.

The project then shows on the other instance under **Settings › Remote projects**, as
"**Valley · remote project of studio**", and on the owner under **Remote instances**.

Pairing is for one project. To let the same instance work on another project, pair it again from that project.

An instance has a project either as its own or as a remote project, never both: joining a project whose repository
it already has as its own is refused, and so is adding a remote project's repository as its own.

## On the owner: Remote instances

Each remote instance shows:

- its **state**: **connected**, **paused** (paused on that instance), **offline** (not heard from for a minute and a
  half), or **needs an update**;
- its version, operating system, CPUs and free disk space;
- the agents it has, and whether each is logged in, and tools such as git, Godot and MSVC;
- when it was last seen.

**Check now** asks each one again. **Remove…** stops it working on the project and revokes the link at both ends;
pairing again needs a new code.

**Needs an update** means the two instances speak different versions of the link. The message says which one to
update, and to which version, for example "update win-box to v0.7.2".

## On the remote instance: Remote projects

**Settings › Remote projects** lists the projects this instance joined, each with its state and when the owner was
last heard from.

- **Open on <owner>** opens the project's board on the owner.
- **Pause** stops this instance taking work for the project; **Resume** starts it again.
- **Leave…** stops working on the project and revokes the link; the owner keeps the project.

A remote project has no board, tickets or settings here, and this instance's own PM, workflows, pickup and Inbox
never see it.

## How the link is kept safe

- The instances talk only over your tailnet, and each checks that a request comes from your Tailscale login.
- Pairing gives each end its own key for that one project, kept in its keychain. A remote instance takes requests for
  a project only from that project's owner, signed with its key.
- Removing or leaving revokes the keys.
