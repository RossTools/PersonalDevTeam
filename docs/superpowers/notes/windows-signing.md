# Windows release signing

Why (PDT-30): every release is a new executable. Unsigned, it has no reputation, and Windows Defender's
machine-learning detections (`Trojan:Win32/Bearfoos.A!ml` on v0.1.161) quarantined the running `pdtd.exe`. An
Authenticode signature from a publicly trusted certificate gives every release the publisher's reputation. A
self-signed certificate does not help.

## How CI signs

`.github/actions/bundle` (job `bundle (windows-latest, nsis)`) with `scripts/sign-windows.ps1`:

1. `pdtd.exe` and `pdt-host.exe` are signed in `target/release` before `stage-sidecars.mjs` copies them.
2. Tauri's `bundle.windows.signCommand` (written to `$RUNNER_TEMP/sign.conf.json`) signs the app exe, the NSIS
   installer and its uninstaller.
3. "Verify the Windows signatures" unpacks the installer with 7-Zip and fails unless the installer and every `.exe`
   in it is validly signed and timestamped.

A main release without the credentials fails at "Windows signing". PR builds are signed when the credentials are
there, else they stay unsigned with a notice.

Signing uses [Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/) (formerly Trusted
Signing): SignTool plus the `Microsoft.ArtifactSigning.Client` NuGet package's dlib, authenticated as an app
registration through environment variables.

## Setup (once)

1. In Azure, create an Artifact Signing account (Basic plan) in a region; note its endpoint
   (e.g. `https://wus2.codesigning.azure.net`).
2. Complete identity validation (Public Trust; an individual developer can validate) and create a **Public Trust**
   certificate profile.
3. Create an app registration (Entra ID) with a client secret, and give it the role
   **Artifact Signing Certificate Profile Signer** on the account (or the profile).
4. In the GitHub repo settings (Secrets and variables → Actions):
   - secrets `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`
   - variables `ARTIFACT_SIGNING_ENDPOINT`, `ARTIFACT_SIGNING_ACCOUNT`, `ARTIFACT_SIGNING_PROFILE`
5. Run the Windows bundle on a PR (label `bundle`) and check "Verify the Windows signatures" passes.

The client secret expires (at most 2 years): renew it before then, or releases fail.

## If Defender flags a release anyway

New certificates also build reputation over time. Report the false positive at
https://www.microsoft.com/wdsi/filesubmission (as a software developer, with the file). Recovering a host:
restore the file from Windows Security → Protection history (or `MpCmdRun.exe -Restore -Name "<threat>" -All`
elevated), or reinstall. pdt-host starts pdtd again as soon as `pdtd.exe` is back; while it is missing, pdt-host
writes `<data dir>/host-problem.json` and the tray shows the message.
