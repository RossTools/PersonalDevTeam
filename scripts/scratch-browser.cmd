@rem The "browser" of a scratch daemon (PDT_BROWSER, set by scratch-daemon.ps1): keeps the pairing link pdtd
@rem opens in the scratch folder, for `scratch-daemon.ps1 pair` to print.
@echo %*>"%PDT_DATA_DIR%\..\pair-url"
