# Devin for Terminal (PDT-54)

Facts PDT relies on, checked against `devin 3000.11.92` on macOS on 2026-10-19 (bundled docs in
`~/.local/share/devin/cli/_versions/current/share/devin/docs`, and live sessions in a scratch repo). Windows is untested.

## Install and login

- Official installer: `curl -fsSL https://cli.devin.ai/install.sh | bash` puts the binary under
  `~/.local/share/devin/cli/_versions/<version>` with a link at `~/.local/bin/devin` (already in PDT's macOS install
  folders) and updates itself. There is also a Homebrew cask (`devin-cli`).
- On the dev Mac the cask's binary hung before running any code (stuck in `_dyld_start`, 0 CPU): macOS had wedged on
  the quarantined first launch, and every launch from that path hung, even after `xattr -c` and a reinstall. A copy
  anywhere else ran. The curl installer avoided it.
- `devin auth status` exits 0 whether or not you are logged in; logged out it prints `Not logged in.`, logged in
  `Logged in (via Devin).`. PDT's host check reads the text. Devin is optional: a missing one is not a warning.

## Launch

- `devin [OPTIONS] [PATH]... [-- <PROMPT>...]`: a bare argument is a folder to work in (`… is neither a known
  subcommand nor an existing path`), so PDT passes the first prompt after `--`, last.
- Bypass: `--permission-mode dangerous` (aliases `yolo`, `bypass`).
- Resume: `--resume <id>` (`-r`). Ids are word pairs like `cool-abstract`.
- Devin saves a session only at its first prompt (`sessions.created_at` is the prompt time). Resuming an id it never
  saved does not fail: it quietly starts a new session with a new id.
- No flag for instructions. Devin reads AGENTS.md, CLAUDE.md, `.devin/rules` etc. from the repo.
- `devin acp` is an Agent Client Protocol server (JSON-RPC on stdio); PDT does not use it yet, so Devin sessions are
  terminal only (`AgentKind::has_conversations`).

## Activity: OSC 7501

Devin's TUI emits the same program-status sequence as Codex, with `app=devin`:

```
ESC ] 7501 ; state=idle:app=devin BEL
ESC ] 7501 ; state=working:app=devin:msg=<base64> BEL
ESC ] 7501 ; state=blocked:kind=permission:app=devin:msg=<base64> BEL   (approval prompt)
ESC ] 7501 ; state=done:app=devin:msg=<base64> BEL                      (turn ended)
```

PDT reads them with pdt-core `program_status(bytes, "devin")`; no hooks are needed for activity. It also sets the
window title (`OSC 0`/`30`, `devin: <title>`) and shell-integration marks (`OSC 133`).

## Hooks and `--config`

- Devin runs Claude-format hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
  `PermissionRequest`, `Stop`, `PostCompaction`, `SessionEnd`) from `.devin/hooks.v1.json`, `.devin/config*.json`,
  `.claude/settings*.json` in the project and `~/.config/devin/config.json`, `~/.claude.json`, `~/.claude/settings*.json`
  for the user. No flag or environment variable adds a hooks file.
- `--config <path>` *replaces* the user config file (`~/.config/devin/config.json`; `%APPDATA%\devin\config.json` on
  Windows), which holds the user's settings, including `devin.org_id`, and is where Devin saves settings back
  ("always allow", theme). It is JSON with comments.
- A hook's `command` runs through a shell; an `args` list is ignored. So PDT quotes the program itself.
- Every payload has `session_id` (stable for the session) and `hook_event_name`. `SessionStart` has `source`
  (`startup` or `resume`) and fires at launch, before the first prompt, with the id the session will be saved under;
  a resumed session reports its own id again.
- A `SessionStart` hook that prints `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"…"}}`
  adds that text to the model's context (verified: the model repeated a word given only there).

## What PDT does

Each Devin terminal gets `--config <data dir>/sessions/<id>/devin-config.json`: the user's own config (parsed as JSONC)
with one `SessionStart` hook appended after the user's own: `<pdtd> hook devin --instructions <session dir>/instructions.md`.
`pdtd hook devin` posts `{source: devin, event, sessionId}` to the session's activity route, which records the id as
`external_session_id` (the latest report wins, since a resumed-but-unsaved session comes back with a new id), and on
`SessionStart` prints the instructions as context. Restart and auto-resume launch `--resume <that id>`.

Checked end to end on a throwaway dev pdtd with the real Devin: the first answer quoted the PDT session id from the
instructions, the conversation id was recorded, activity went idle → working → idle, and after kill + restart Devin
resumed the conversation and recalled the earlier request. A new folder shows Devin's own trust dialog first.

If the user's config cannot be read or parsed, Devin starts without `--config` (no PDT hook, so no resume id and no
instructions) rather than on a copy that lost the user's settings.

Known cost: settings Devin saves during a PDT session go to PDT's copy, not the user's file; the next launch copies the
user's file again. The user chose this over no hooks (no exact resume, no PDT instructions).
