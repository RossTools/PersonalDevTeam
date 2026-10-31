# Your data

Where PDT keeps its data and logs, how it backs up its database, and how to move or remove it.

Everything PDT knows stays on your machine: tickets, conversations, workflows, settings. Nothing goes to a PDT
server (there isn't one). Your code stays in your repositories; the agents you run send what they read to their own
providers, as they would outside PDT.

## Where it is

PDT keeps everything in one data folder:

| System | Data folder |
| --- | --- |
| macOS | `~/Library/Application Support/pdt` |
| Windows | `%LOCALAPPDATA%\pdt` |
| Linux | `~/.local/share/pdt` (or `$XDG_DATA_HOME/pdt`) |

The main things in it:

- `pdt.db`: the database, with your projects, tickets, sessions, workflows and settings (and `pdt.db-wal` and
  `pdt.db-shm` beside it while PDT runs);
- `backups/`: copies of the database (below);
- `logs/`: PDT's logs (below);
- `sessions/`, `pm/` and `runs/`: what PDT keeps for agent sessions, the PM and runs (a run's own data folder, its
  log);
- `network.json`: the port and allowed hosts from **Settings › This host › Network**;
- `update/`: downloads and the database copy taken before an update;
- on Linux without a system keyring, `secrets.json` (see [Secrets](secrets.md#where-values-are-kept)).

Your secrets, the GitHub login and the update token are in your system's keychain, not in the folder. Your
repositories and their worktrees stay where they are, in your projects folder.

## Disk space

Each ticket works in its own worktree, and each worktree builds on its own: a Rust `target` folder or `node_modules`
can take gigabytes per worktree. Worktrees stay after a ticket is done, so PDT cleans up after them.

- **Settings › This host › Storage** shows how much is free on each disk PDT uses and what each worktree takes,
  build output apart. Sizes are measured in the background every half hour; **Measure again** does it now.
- **Clean** removes a worktree's build output (the project's [build output folders](../automation/run-configuration.md#storage)
  that git ignores), never your code or uncommitted work. Its next build starts from scratch.
- **Clean all finished** cleans every Done or Won't do ticket's worktree at once.
- On its own, PDT cleans a finished ticket's worktree two days after it is done. Change that per project in its
  **Worktrees** settings with **Clean build output after**, and set **Remove worktree after** to remove the worktree
  itself too (its branch stays; a worktree with uncommitted changes is kept). Worktrees a ticket still works in, and
  ones that belong to no ticket, are never touched.
- When a disk has less than 20 GB free, the Inbox says so. Below 5 GB, auto-start starts no new tickets until there is
  more space; agents already running carry on. Change both in **Settings › This host › Storage** (0 turns one off).

## Logs

The logs are in the `logs` folder of the data folder:

- `pdtd.<date>.log`: PDT's own log, one per day, the last 14 kept. Start here.
- `pdt-host.log`: the supervisor's log: starts, stops, updates and rollbacks.
- `pdtd-launch.log`: what PDT printed as it started, before its own log opened. Look here when PDT won't start.
- `pdt-host.launchd.log` (macOS): the supervisor's output as macOS started it.

On Linux the supervisor's output also goes to `journalctl --user -u pdt-host`.

Secret values never appear in the logs (PDT writes `[secret:NAME]` instead). **Report a bug…** can attach the log's
last lines, cleaned of tokens and your machine's names.

## Backups

PDT copies its database before every update, and puts it back if the update rolls back. It also copies it into
`backups/` before it changes the database's layout, keeping the last three copies.

For a backup of your own while PDT runs, use SQLite's backup command (a plain copy of a database in use can be
inconsistent):

```sh
cd ~/Library/Application\ Support/pdt
sqlite3 pdt.db ".backup pdt-backup.db"
```

Or let your usual backup tool (Time Machine and the like) include the data folder.

Old history is pruned as PDT goes: finished workflow runs, ended Inbox items, notifications and the event log after
30 days. Tickets, comments and conversations' messages stay.

## Move to another machine

PDT's data folder works on another machine with the same system, provided your repositories are at the same paths
there. Secrets and logins don't move with it: they're in the old machine's keychain.

1. On the old machine, [uninstall PDT](updates.md#uninstall). That stops it and leaves the data folder
   alone.
2. Copy the data folder to the same place on the new machine.
3. Clone your repositories to the same paths as on the old machine.
4. Install PDT on the new machine. It finds the data folder and opens with your projects and tickets.
5. Sign in to GitHub again, save the update token if you had one, and give your secrets their values again:
   **Settings › Secrets** still lists them; **Edit** each and paste its value.

Moving between systems (from a Mac to Windows, say) isn't supported: the paths in the data differ.

## Remove everything

[Uninstall PDT](updates.md#uninstall) first, then delete the data folder. To remove the secrets too,
delete them in **Settings › Secrets** before you uninstall, or remove the entries named for Personal Dev Team in your
keychain (Keychain Access on macOS, Credential Manager on Windows), under `dev.personaldevteam`.
