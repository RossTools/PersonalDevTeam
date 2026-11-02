#!/usr/bin/env bash
# The "browser" of a scratch daemon (PDT_BROWSER, set by scratch-daemon.sh): keeps the pairing link pdtd opens in the
# scratch folder, for `scratch-daemon.sh pair` to print.
printf '%s\n' "$1" >"$PDT_DATA_DIR/../pair-url"
