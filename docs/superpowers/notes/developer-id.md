# macOS signing: why updates still prompt, and moving to a Developer ID

## The problem (found 2026-10-17, v0.1.61 → v0.1.73)
Every macOS update makes pdtd's read of the update token wait on a keychain "allow access" prompt, although every
release is signed with the same self-signed certificate and fixed identifiers (`scripts/sign-macos.sh`).

A keychain item has two checks:
- **Access rule** (ACL): which programs may read it, by designated requirement. Ours is
  `identifier "<bundle id>.pdtd" and certificate leaf = H"<cert hash>"`, which every signed release meets.
- **Partition list** (macOS 10.12+): an extra list of partition IDs. Apple-signed code is `apple:`, code signed with a
  Team ID is `teamid:<id>`, and anything else, a self-signed certificate included, is `cdhash:<hash of that exact
  binary>`. Each Always Allow adds the new build's cdhash, so the next build is unknown again and the keychain asks.

`security dump-keychain -a` on the stable Mac showed the item's partition list as three `cdhash:` entries, none of
them v0.1.73's pdtd. The 2026-10-17 incident's reproduction ("a rebuild signed with the same certificate reads
without a prompt") re-signed a binary with the same code hash, so it never tested a new build.

Only a certificate with a Team ID fixes this: an Apple **Developer ID Application** certificate. The partition list
then holds `teamid:<id>`, which every release matches. It also lets releases be notarized, so Gatekeeper stops
warning about an unidentified developer.

## Moving to a Developer ID (once)
1. Enroll in the Apple Developer Program (developer.apple.com/programs, US$99/year).
2. Create a **Developer ID Application** certificate: Xcode → Settings → Accounts → Manage Certificates → + →
   Developer ID Application (or Certificates, IDs & Profiles on developer.apple.com with a CSR from Keychain Access).
3. Export it with its private key from Keychain Access as a `.p12` with a strong password, and replace the GitHub
   secrets (repo → Settings → Secrets and variables → Actions):
   - `MACOS_SIGNING_P12`: `base64 -i DeveloperID.p12 | pbcopy`
   - `MACOS_SIGNING_P12_PASSWORD`: its password

   `scripts/sign-macos.sh` sees the `Developer ID Application:` name and signs with the hardened runtime and a secure
   timestamp. Keep the old self-signed `.p12` until the switch is done, then delete it.
4. For notarization, create an App Store Connect API key (appstoreconnect.apple.com → Users and Access →
   Integrations → App Store Connect API, role Developer) and add:
   - `MACOS_NOTARY_KEY`: `base64 -i AuthKey_<id>.p8 | pbcopy`
   - `MACOS_NOTARY_KEY_ID`: the key id
   - `MACOS_NOTARY_ISSUER`: the issuer id

   With these set, releases on main run `scripts/notarize-macos.sh` (submit, wait, staple). Without them releases are
   signed but not notarized.
5. Merge anything to main. Install the release it publishes, then on each Mac answer the keychain prompt once with
   **Always Allow** (or delete the `update-token` item in Keychain Access, restart the daemon from the tray and save
   the token again). Later releases read the token without a prompt.
6. Check after the next update: Host settings shows the token, and `security dump-keychain -a` lists `teamid:` in
   the item's partition list.
