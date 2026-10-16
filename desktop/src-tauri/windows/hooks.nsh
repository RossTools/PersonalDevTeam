; Registers pdt-host to start at login and starts it (it starts pdtd). During an update pdt-host is already running
; from the data folder and only refreshes the registration.
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\pdt-host.exe" install --install-dir "$INSTDIR"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$INSTDIR\pdt-host.exe" uninstall'
!macroend
