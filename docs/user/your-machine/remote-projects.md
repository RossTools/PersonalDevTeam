# Remote projects

Let another instance of PDT work on one of your projects, with the board, tickets and PM staying where they are.

Each PDT you install is an **instance** with a name, like `studio` or `win-box`. A project belongs to one instance,
its **owner**, which keeps its board, tickets, workflows, Inbox and PM. Other instances can join it as **remote
instances** to work on it for the owner, for example to build and test on another operating system. Each instance
keeps its own projects as usual.

A ticket that has to run elsewhere (a Windows client, say) runs on a remote instance: its branch and worktree are
there, and so are its agents. You follow and steer it from the owner as if it ran there: its sessions are under the
ticket, marked **on win-box**.

## Before you start

- **Name each instance** in **Settings › This host › Name** (or in **Get started**), so you can tell them apart.
- **Turn on remote access on both**, with the same Tailscale owner. See
  [Remote access and your phone](remote-access.md). The two instances reach each other through it.

## Add a remote instance to a project

1. **On the owner**, open the project's settings, then **Remote instances**, and click **Add**. PDT shows a pairing
   code and this instance's address. The code works once, for 10 minutes.
2. **On the other instance**, click **Add project**, choose **Join a remote**, and enter the owner's address and the
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
- when it was last seen;
- **At most … tickets in flight on it at once**: its cap for this project (below).

**Check now** asks each one again. **Remove…** stops it working on the project and revokes the link at both ends;
pairing again needs a new code. If the instance can't be reached (asleep, or off the tailnet), it is removed here all
the same and PDT says it couldn't be told: once it is back and asks, its remote project shows **removed by its owner**,
and you leave it there to finish. Removing the project itself does the same for each of its remote instances.

An instance that left the project while the owner couldn't be reached says so when it is checked: "win-box left the
project; remove it here too".

**Needs an update** means the two instances speak different versions of the link. The message says which one to
update, and to which version, for example "update win-box to v0.7.2". You can update it from the owner: see
[Updating remote instances](updates.md#updating-remote-instances).

## Choose where tickets run

On the owner, under the project's **Remote instances**, **Placement** maps a label to an instance: with
`windows → win-box`, every ticket labelled `windows` runs on win-box. Tickets with no matching label run on the owner,
as before. The PM suggests the label when it drafts a ticket whose work has to run somewhere in particular.

The repository can say what a kind of ticket needs instead of naming an instance, so the rule works on anyone's
instances. In `.pdt/project.toml`:

```toml
[[placement]]
when = { labels = ["windows"] }
needs = { os = "windows", tools = ["godot"] }
```

A `windows` ticket then runs on a joined instance that runs Windows and has Godot, unless the owner itself does.

**Start…** has a **Runs on** choice when the project has remote instances: the project's rules (the default), this
instance, or one of the remote instances. Placement goes in this order: **Runs on**, then the project's label rules,
then the repository's needs, then the owner. A ticket placed on an instance that is offline or paused doesn't start;
Start says why.

## Auto-start across instances

With auto-start on, the owner and each remote instance take tickets side by side, each up to its own cap:

- **The owner** takes up to the project's **Running tickets at once**, counting only the tickets running on it.
- **Each remote instance** takes up to its cap under **Remote instances**. Its own **Remote projects** page can set
  a cap too; the lower of the two applies. With neither set, it takes one at a time.

A ticket placed on an instance that is **offline** or **paused** waits in **To do**. Its card says "waiting for
win-box (offline)", and it starts once win-box is back. It never goes to another instance instead. If win-box stays
offline for two hours with work waiting, the owner sends a notification; it clears once win-box is back.

A ticket whose repository needs something no instance has, the owner included, waits with "no instance can run this
(needs os=windows, godot)". Join an instance that has it, or change the ticket's labels.

The **on win-box** mark on a ticket's sessions says **offline** while the owner can't reach it; hover it for when it
was last seen.

## What runs on the remote instance

For a ticket placed there:

- **The checkout.** The remote instance clones the repository from the owner's `origin` the first time, into the
  folder its **Remote projects** page shows, and makes the ticket's worktree there. It needs your git credentials for
  fetching and pushing; the pull request, CI and merge stay with the owner.
- **The agents.** The Coder, Reviewer and QA run there as conversations, in that worktree. On the owner they are under
  the ticket and fully interactive: you read and answer them, approve their requests, interrupt, stop and restart
  them, and they count in the Inbox, notifications and the board like any other.
- **PDT's tools.** The agents' PDT tools (`submit_outcome`, `get_ticket`, and the rest) are answered by the owner, so
  confirmations and cards work as usual.
- **Git.** Bringing the branch up to date with its base at merge time, and removing the worktree when the ticket
  finishes, happen there too.
- **Runs and tests.** Those you start in the ticket's worktree run there, from its `.pdt/project.toml`, and only those
  for its operating system are offered (`os` in Run configuration). You start, stop and restart them on the owner,
  where they show marked **on win-box** with their state and output. A web run's link is the remote instance's own,
  through its remote access, so **Open** works from the owner (and only for you). They count toward the remote
  instance's maximum of runs (its **Settings › This host › Runs**), not the owner's. If the remote instance
  restarts, its runs stop.
- **Files.** In **Files**, pick the ticket's branch (marked **on win-box**) to browse, read, edit, search and download
  its files on the remote instance, as you would locally.
- **Changes.** The ticket's **Changes** show its diff on the remote instance, with review comments, **Send feedback**
  and **Approve** as usual. Merge checks (commits to merge, conflicts with the base) are done there; a remote ticket
  merges through its pull request.
- **Overlaps.** The files a remote ticket changes count in overlap checks with the project's other tickets, wherever
  those run.
- **Terminals.** An agent started in its terminal, and any shell you open in the ticket's workspace, runs there too.
  You type, resize and scroll back on the owner as in a local terminal; the remote instance keeps the output while
  nobody watches, so reconnecting shows it.

Over a slower link, opening a ticket's Changes fetches the whole diff at once, so moving between its files doesn't
wait on the remote instance again, and a diff that hasn't changed isn't fetched again.

The local forge's CI isn't available for a remote ticket yet. For secrets on a remote instance, see
[Secrets on a remote instance](#secrets-on-a-remote-instance). A remote session keeps the view it started in: it can't switch
between conversation and terminal.

## Secrets on a remote instance

Agents on a remote instance use secrets as they do on the owner (see [Secrets](secrets.md)), with **run with
secrets** and **write a secret file** running there, in the ticket's worktree:

- **Your secrets on the owner** work there by default. The owner sends a secret's value over the encrypted link for
  that one command or file, and only after you allow it as usual. The remote instance never stores or logs it, and the
  command's output comes back with the value replaced by `[secret:NAME]`.
- **Secrets kept on the remote instance** are for what belongs to that machine: a code-signing certificate, a token
  for a tool signed in there. Add them on the remote instance, under the project in **Settings › Remote projects**.
  They stay in its keychain and are never sent to the owner or anywhere else.
- **On a name clash**, the remote instance's own secret wins, and the owner doesn't send its value.

The owner's **Settings › Secrets** lists both: its own, and under **On remote instances** the names of those kept on
each one, as "on win-box (VAL)", never their values. Every use is recorded: a secret sent from the owner shows **on
win-box** in its **Uses**, and a use of one kept on the remote instance shows there and, by name, on the owner.

## Shells on a remote instance

To work in the remote instance's checkout of the project yourself, open a new tab (**+**) with the project's
repository as the place and choose **Shell on win-box**. A shell in a remote ticket's workspace opens in its worktree
there by itself. Such a shell is a tab like any other, marked **on win-box**, and reopens when you restart it.

A shell from the owner opens only in the remote project's checkout or its tickets' worktrees on that instance, never
anywhere else on it. Shells you open in the remote instance's own window stay its own and never show on the owner.

If the link between the instances drops, the agents keep working, and so do terminals: the terminal on the owner
reconnects once the link is back. Files and Changes of their tickets say the remote instance can't be reached until it
is back. Their PDT tool calls wait and go through once it is
back, and everything they said meanwhile arrives then. If the owner restarts, the remote instance keeps what they said
until it is back. If the remote instance restarts, its agents stop; sending one a message starts it again where it
left off.

## On the remote instance: Remote projects

**Settings › Remote projects** lists the projects this instance joined, each with its state and when the owner was
last heard from.

- **Open on <owner>** opens the project's board on the owner.
- **Pause** stops this instance taking work for the project; **Resume** starts it again.
- **Leave…** stops working on the project and revokes the link; the owner keeps the project. If the owner can't be
  reached, PDT says so: remove this instance from the project on the owner too.
- **Removed by its owner** means the owner removed this instance, or the project, while the two couldn't reach each
  other. Nothing more comes from the owner; **Leave…** to finish.
- **Checkout** is the folder the project's checkout and worktrees go in. Change it before the first ticket runs here;
  a checkout already made stays where it is.
- **Secrets** kept here for the project: the owner's agents here use them, and they never leave this machine. Add one
  with its name and value; **Remove…** takes it off the project.
- **Update channel**: when the owner updates this instance, it checks on the owner's channel unless you switch on
  using this host's own.
- **At most … of its tickets here at once** caps how many of the project's tickets run here at once. Leave it blank
  to let the owner's cap decide.

It also lists what runs here for the owner now, read-only: each agent's or shell's ticket, title and how long it has
run.

A remote project has no board, tickets or settings here, and this instance's own PM, workflows, pickup and Inbox
never see it.

## How the link is kept safe

- The instances talk only over your tailnet, and each checks that a request comes from your Tailscale login.
- Pairing gives each end its own key for that one project, kept in its keychain. A remote instance takes requests for
  a project only from that project's owner, signed with its key.
- Removing or leaving revokes the keys.
