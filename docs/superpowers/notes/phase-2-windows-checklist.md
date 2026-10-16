# Phase 2 hand tests on Windows (update path)

The Windows counterpart of `phase-2-mac-mini-checklist.md` §5, for what only a real NSIS install shows. Record PASS
or FAIL (with what you saw) after each step. Every step that installs, updates, stops or rolls back the stable app or
the stable pdtd is **USER does this**: agents never touch the stable instance; they ask the user, then observe and
record. Paths below: `%LOCALAPPDATA%\pdt` is the stable data folder; the install folder is the one the installer
chose (default `%LOCALAPPDATA%\Personal Dev Team`).

## 0. Preconditions
1. The stable app is installed and running: `curl.exe -s http://127.0.0.1:7420/api/health` answers (reading health is
   allowed). Note the version.
2. pdt-host runs from the data folder, not the install folder:
   `Get-Process pdt-host | Select-Object Id, Path` → `Path` is `%LOCALAPPDATA%\pdt\host\pdt-host.exe`.
3. The tray app is running (its icon is in the notification area).
4. `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run` lists the pdt-host entry pointing at the data-folder
   copy. Reading the registry is allowed; changing it is not.

## 1. Silent update over a running data-folder pdt-host
1. **USER does this** (updates the stable app): with a release newer than the installed one available (CI, or
   `scripts\release-stable.ps1` run by the user), click Update in Settings → This host or the tray.
2. You observe:
   - `%LOCALAPPDATA%\pdt\logs\pdt-host.log` shows `installing <new> (from <old>)` and then `v<new> is healthy`
     (the installer runs silently with `/S /D=<install folder>`).
   - The same pdt-host process id as in 0.2 is still running (the installer never stopped it), unless the release
     changed pdt-host itself, in which case the log says "replacing and restarting" and a new process runs from the
     same data-folder path.
   - `curl.exe -s http://127.0.0.1:7420/api/health` reports the new version; `%LOCALAPPDATA%\pdt\update\result.json`
     has outcome `installed`; the banner says "Updated to v…".

## 2. `$UpdateMode` and the PREUNINSTALL hook
1. During step 1 the NSIS installer runs the old uninstaller with `/UPDATE`. You check afterwards: the Run key entry
   from 0.4 is still there, and pdt-host was not stopped by `pdt-host uninstall` (the log has no "uninstalling" line
   at the update's time). This is `desktop/src-tauri/windows/hooks.nsh`: PREUNINSTALL skips `uninstall` when
   `$UpdateMode` is 1.
2. POSTINSTALL ran `pdt-host install`: the log has no `registering pdt-host failed` line, and the Run key still points
   at the data-folder copy.

## 3. The tray after an update
1. You observe: the tray icon disappeared during the install (pdt-host stops it, because it locks files) and came
   back by itself afterwards, from the install folder (`Get-Process` → its `Path` is in the install folder), with
   the new version in its menu.

## 4. Rollback
1. **USER does this** (forces a rollback of the stable app): prepare an installer whose pdtd fails its health check
   (for example build one with a pdtd that only sleeps), write `%LOCALAPPDATA%\pdt\update\request.json` with
   `{"version":"9.9.9","tag":"v9.9.9","asset":"<that setup.exe>","fromVersion":"<current>","requestedAt":0}`, then
   `curl.exe -X POST http://127.0.0.1:7420/api/shutdown`.
2. You observe: within ~70 s the previous version answers health again; `result.json` has outcome `rolled_back`;
   the banner shows "Update to v9.9.9 failed and was rolled back"; `pdt-host.log` has
   `v9.9.9 failed (…); rolling back`; the install folder matches its copy in `%LOCALAPPDATA%\pdt\update\previous`;
   the tray is running again. The same version is not offered again automatically.
3. Known: the uninstall registry entry (Apps & features) keeps showing 9.9.9 after the rollback (carry-over note).
