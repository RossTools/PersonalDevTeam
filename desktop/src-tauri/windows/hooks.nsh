; Registers pdt-host to start at login and starts it (it starts pdtd). During an update pdt-host is already running
; from the data folder and only refreshes the registration.
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\pdt-host.exe" install --install-dir "$INSTDIR"'
!macroend

; Stops this install's pdt-host and pdtd (so the folder can be removed) and removes the login registration. Skipped
; when the uninstaller runs as part of an update (/UPDATE), which must leave the supervisor running.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\pdt-host.exe" uninstall --install-dir "$INSTDIR"'
  ${EndIf}
!macroend
