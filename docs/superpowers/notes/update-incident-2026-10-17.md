# Update incident: v0.1.35 → v0.1.42 on the Mac mini (2026-10-17)

## What happened
1. **pdtd hung at startup on the keychain.** Releases were ad-hoc signed, so every build is a different code identity
   (v0.1.35's pdtd CDHash `886ae8…`, v0.1.42's `364b14…`; identifier `pdtd-<hash>` from the linker). The update token
   was stored by v0.1.35, and the keychain trusts only the program that stored it, so v0.1.42's read waited on an
   "allow access?" prompt no one could see. pdtd read the token after binding its port and before serving, so
   `/api/health` never answered and pdt-host rolled back. Reproduced on the Mac mini with a throwaway keychain: an
   ad-hoc rebuild blocks; a rebuild signed with the same certificate and identifier reads the item without a prompt.
2. **The rollback could not start.** v0.1.42 had applied migration 7; pdt-host restored v0.1.35's app but not the
   database, and v0.1.35 refuses a database with a migration it does not know (`migration 7 was previously applied
   but is missing`). Recovery was by hand: reinstall v0.1.42, and remove the update token.

## Fixes
- Keychain calls are time-limited (5 s, `secrets::TimeLimited`); while one is stuck the next fail at once with
  `keychain_unavailable` / an update `error` saying macOS may be waiting for approval. The startup token read runs
  in the background, so serving and the health check never wait on the keychain.
- pdt-host copies the database (`pdt.db`, `-wal`, `-shm`) to `update/db-before-update` with pdtd stopped, before
  installing, and puts it back whenever it rolls back (after a failed health check or an interrupted install).
  Writes the failed version made in its few seconds are lost; a successful update drops the copy.
- macOS releases are signed with one self-signed certificate and fixed identifiers (`scripts/sign-macos.sh`; CI
  secrets `MACOS_SIGNING_P12`, `MACOS_SIGNING_P12_PASSWORD`, made by `scripts/make-signing-cert.sh`). CI refuses to
  publish an unsigned release. Gatekeeper still sees an unidentified developer (a Developer ID would fix that).
  **Correction:** this does not keep the keychain's trust across updates. The keychain's partition list trusts a
  self-signed program by its exact code hash, so every new release asks again; only a Developer ID (Team ID) fixes
  it. See docs/superpowers/notes/developer-id.md.

The one-time move of the Mac mini onto signed releases is done (steps in git history).
