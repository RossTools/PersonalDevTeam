#!/bin/sh
# Installs Personal Dev Team (PDT) on macOS or Linux, without sudo:
#
#   curl -fsSL https://pdt.dev/install.sh | sh
#   curl -fsSL https://pdt.dev/install.sh | sh -s -- --channel nightly
#
# Options (or the environment variables beside them):
#   --channel stable|nightly  PDT_CHANNEL   the newest stable release (the default), or the newest nightly build
#   --version X.Y.Z           PDT_VERSION   an exact release
#   --force                   PDT_FORCE=1   install again over an existing install
#   --yes                     PDT_YES=1     answer yes to questions (Linux: keep PDT running after you log out)
#   GITHUB_TOKEN                            optional: only for a private fork, or when GitHub's rate limit is used up
#
# It downloads the release for this system from GitHub (without GITHUB_TOKEN, a stable release takes no call to
# GitHub's API, a nightly one call), checks that the release's SHA256SUMS carries PDT's release signature
# (SHA256SUMS.sig, checked with openssl against the key below) and the download against SHA256SUMS, and installs it:
# macOS into ~/Applications (then opens it once, which starts it at login), Linux into ~/.local/opt/pdt as a systemd
# user service. Windows has install.ps1.
#
# Everything runs from pdt_install, called on the last line: a download cut short runs nothing.
#
# For tests only: PDT_INSTALL_API (GitHub's API, https://api.github.com), PDT_INSTALL_WEB (where releases download
# from, https://github.com), PDT_INSTALL_LOCAL (where a running PDT answers, http://127.0.0.1:7420),
# PDT_INSTALL_SYSTEM_APPS (/Applications) and PDT_INSTALL_SIGNING_KEY (the key below).

set -eu

pdt_install() {
	repo="RossTools/PersonalDevTeam"
	product="Personal Dev Team"
	slug="pdt"
	port=7420
	host_label="dev.personaldevteam.host"
	host_unit="pdt-host.service"
	site="https://pdt.dev"
	# The release signing key (base64 of its public key's DER; the same as RELEASE_SIGNING_KEY in PDT's source). Empty
	# until releases are signed: then SHA256SUMS alone is checked.
	signing_key=""
	signing_key="${PDT_INSTALL_SIGNING_KEY:-$signing_key}"
	api="${PDT_INSTALL_API:-https://api.github.com}"
	web="${PDT_INSTALL_WEB:-https://github.com}"
	local_url="${PDT_INSTALL_LOCAL:-http://127.0.0.1:$port}"
	sys_apps="${PDT_INSTALL_SYSTEM_APPS:-/Applications}"
	channel="${PDT_CHANNEL:-stable}"
	version="${PDT_VERSION:-}"
	force="${PDT_FORCE:-}"
	yes="${PDT_YES:-}"

	while [ $# -gt 0 ]; do
		case "$1" in
		--channel) [ $# -ge 2 ] || die "--channel needs a value: stable or nightly"; channel="$2"; shift 2 ;;
		--channel=*) channel="${1#*=}"; shift ;;
		--version) [ $# -ge 2 ] || die "--version needs a value, like 0.3.0"; version="$2"; shift 2 ;;
		--version=*) version="${1#*=}"; shift ;;
		--force) force=1; shift ;;
		-y | --yes) yes=1; shift ;;
		-h | --help) usage; return 0 ;;
		*) die "unknown option: $1 (see --help)" ;;
		esac
	done
	case "$channel" in stable | nightly) ;; *) die "--channel is stable or nightly, not '$channel'" ;; esac
	version="${version#v}"
	if [ -n "$version" ]; then
		printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' ||
			die "--version is a release version like 0.3.0, not '$version'"
	fi

	need curl
	need tar
	need mktemp
	if command -v sha256sum >/dev/null 2>&1; then
		sha256() { sha256sum "$1" | cut -d' ' -f1; }
	elif command -v shasum >/dev/null 2>&1; then
		sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
	else
		die "this needs sha256sum or shasum to check the download, and found neither"
	fi
	if [ -n "$signing_key" ] && ! command -v openssl >/dev/null 2>&1; then
		die "this needs openssl to check the release's signature. Install openssl and run it again."
	fi

	# The system, and the release asset for it (the names hosts update from: pdt_core::release::select_asset).
	os="$(uname -s)"
	arch="$(uname -m)"
	case "$os" in
	Darwin)
		# One universal app for Apple silicon and Intel Macs.
		os=mac
		;;
	Linux)
		os=linux
		case "$arch" in
		x86_64 | amd64) arch=x86_64 ;;
		aarch64 | arm64) arch=aarch64 ;;
		*) die "$product for Linux is built for x86_64 and aarch64, not $arch." ;;
		esac
		check_glibc
		;;
	MINGW* | MSYS* | CYGWIN* | Windows*)
		die "On Windows, install $product from PowerShell: irm $site/install.ps1 | iex"
		;;
	*) die "$product runs on macOS, Linux and Windows, not $os." ;;
	esac

	# An install counts as one only when it starts at login (its LaunchAgent or systemd user unit, as `pdt-host install`
	# writes them) or runs: files left by an install cut short are installed again.
	if [ "$os" = mac ]; then
		app_name="$product.app"
		dest="$HOME/Applications/$app_name"
		existing=""
		for d in "$HOME/Applications/$app_name" "$sys_apps/$app_name"; do
			if [ -d "$d" ]; then existing="$d"; break; fi
		done
		login_item="$HOME/Library/LaunchAgents/$host_label.plist"
		start_cmd="open \"$existing\""
	else
		dest="$HOME/.local/opt/$slug"
		data="${XDG_DATA_HOME:-$HOME/.local/share}/$slug"
		existing=""
		for d in "$dest" "$data/app"; do
			if [ -e "$d/$slug-host" ]; then existing="$d"; break; fi
		done
		login_item="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$host_unit"
		start_cmd="systemctl --user start $host_unit"
	fi
	registered=""
	if [ -n "$existing" ] && [ -e "$login_item" ]; then registered=1; fi
	running=""
	if curl -fsS --max-time 3 "$local_url/api/health" 2>/dev/null | grep -q "\"$product\""; then running=1; fi
	if [ -z "$force" ]; then
		if [ -n "$running" ]; then
			if [ -n "$existing" ]; then say "$product is already installed in $existing, and running."; else say "$product is already running at $local_url."; fi
			say "It keeps itself up to date: Settings > This host > Updates shows its version and checks for a new one."
			say "To install it again anyway, run this with --force."
			return 0
		fi
		# macOS: opening the app registers it, so one this script can't replace (/Applications, usually) is just opened.
		if [ -n "$registered" ] || { [ "$os" = mac ] && [ -n "$existing" ] && [ ! -w "$(dirname "$existing")" ]; }; then
			say "$product is installed in $existing, but it isn't running."
			say "Start it with: $start_cmd"
			say "To install it again instead, run this with --force."
			return 0
		fi
		if [ -n "$existing" ]; then say "$product is in $existing but isn't set up to start at login: installing it again."; fi
	fi
	# Linux: pdt-host runs as a systemd user service, so without a systemd user instance nothing is downloaded.
	if [ "$os" = linux ] && ! systemctl --user show-environment >/dev/null 2>&1; then
		die "$product for Linux runs as a systemd user service, and this session has no systemd user instance. On WSL, turn systemd on: add the lines [boot] and systemd=true to /etc/wsl.conf, run wsl --shutdown in Windows, then open WSL and run this again. In a container, use one that runs systemd. Nothing was installed."
	fi
	# The app is replaced where it is; one in a folder only an admin can change (/Applications, usually) is left to the
	# user rather than asking for sudo (only --force gets here with one).
	if [ "$os" = mac ] && [ -n "$existing" ]; then
		dest="$existing"
		[ -w "$(dirname "$dest")" ] ||
			die "$product is installed in $dest, which needs admin rights to replace. It updates itself (Settings > This host > Updates); to reinstall, move it to the Bin first."
	fi

	# Checked before downloading: the install goes in beside $dest, so its folder (or the nearest one that exists) must
	# be writable.
	parent="$(dirname "$dest")"
	while [ ! -e "$parent" ]; do parent="$(dirname "$parent")"; done
	[ -d "$parent" ] && [ -w "$parent" ] ||
		die "$product installs into $dest, but you can't write to $parent. Make it yours (or writable) and run this again. Nothing was installed."

	tmp="$(mktemp -d)"
	# Removes only the folder made above.
	trap 'rm -rf "$tmp"' EXIT
	trap 'exit 130' INT TERM

	say "Finding the release..."
	# Without a token, GitHub's API is used only to list the nightly builds (an IP has 60 anonymous API calls an hour):
	# the newest stable release's tag comes from github.com's releases/latest redirect, and the files download by name
	# from github.com, which has no such limit. With one, everything goes through the API (a private fork needs that).
	if [ -z "${GITHUB_TOKEN:-}" ]; then
		if [ -n "$version" ]; then
			tag="v$version"
		elif [ "$channel" = nightly ]; then
			fetch_json "/repos/$repo/releases?per_page=30" "$tmp/releases.json" || no_releases
			tag="$(nightly_tag <"$tmp/releases.json")" || true
		else
			tag="$(latest_tag)" || no_releases
		fi
	else
		if [ -n "$version" ]; then
			fetch_json "/repos/$repo/releases/tags/v$version" "$tmp/release.json" ||
				die "there is no $product release v$version (the releases: https://github.com/$repo/releases)."
		elif [ "$channel" = nightly ]; then
			fetch_json "/repos/$repo/releases?per_page=30" "$tmp/releases.json" || no_releases
			tag="$(nightly_tag <"$tmp/releases.json")" || true
			[ -n "$tag" ] || no_nightly
			fetch_json "/repos/$repo/releases/tags/$tag" "$tmp/release.json" || no_releases
		else
			fetch_json "/repos/$repo/releases/latest" "$tmp/release.json" || no_releases
		fi
		tag="$(field tag_name <"$tmp/release.json" | head -n 1)"
		[ -n "$tag" ] || die "GitHub's answer about the release could not be read."
		assets <"$tmp/release.json" >"$tmp/assets"
	fi
	if [ -z "$tag" ]; then no_nightly; fi

	# SHA256SUMS lists the release's files: the one for this system is picked from it.
	if ! download SHA256SUMS "$tmp/SHA256SUMS"; then
		if [ "$http" = 404 ] && [ -n "$version" ]; then
			die "there is no $product release v$version (the releases: https://github.com/$repo/releases)."
		fi
		[ "$http" = 404 ] && die "release $tag has no SHA256SUMS, so the download can't be checked. Nothing was installed."
		download_failed SHA256SUMS
	fi
	if [ -n "$signing_key" ]; then
		if ! download SHA256SUMS.sig "$tmp/SHA256SUMS.sig"; then
			[ "$http" = 404 ] &&
				die "release $tag is not signed (it has no SHA256SUMS.sig), so its SHA256SUMS can't be trusted. Nothing was installed."
			download_failed SHA256SUMS.sig
		fi
		verify_signature ||
			die "the signature of $tag's SHA256SUMS does not match $product's release key. Nothing was installed."
		say "Signature verified."
	fi
	if [ "$os" = mac ]; then
		asset="$(sum_names | grep -E '.\.app\.tar\.gz$' | head -n 1)" || true
		[ -n "$asset" ] || die "release $tag has no download for this system."
	else
		asset="$slug-linux-$arch.tar.gz"
	fi

	say "Downloading $product $tag ($asset)..."
	if ! download "$asset" "$tmp/$asset" --progress-bar; then
		[ "$http" = 404 ] && die "release $tag has no download for this system ($asset)."
		download_failed "$asset"
	fi
	want="$(awk -v n="$asset" '{ f = $2; sub(/^\*/, "", f) } f == n { print tolower($1); exit }' "$tmp/SHA256SUMS")"
	[ -n "$want" ] || die "SHA256SUMS of $tag does not list $asset, so it can't be checked. Nothing was installed."
	got="$(sha256 "$tmp/$asset")"
	[ "$got" = "$want" ] ||
		die "the checksum of $asset does not match SHA256SUMS (expected $want, got $got). Nothing was installed."
	say "Checksum verified."

	mkdir -p "$tmp/unpack"
	tar -xzf "$tmp/$asset" -C "$tmp/unpack"
	if [ "$os" = mac ]; then
		install_mac
	else
		install_linux
	fi

	say ""
	say "$product $tag is installed and starts when you log in."
	say "Open it at $local_url, then connect an agent: $site/docs/getting-started/connect/"
	case "$tag" in *-nightly.*)
		say "To keep getting nightly builds, choose Settings > This host > Updates > Update channel > Nightly builds too."
		;;
	esac
}

install_mac() {
	[ -d "$tmp/unpack/$app_name" ] || die "$asset does not hold $app_name."
	# curl sets no quarantine flag, so Gatekeeper never looks at this app: check its signature here instead.
	if ! codesign --verify --deep --strict "$tmp/unpack/$app_name" 2>"$tmp/codesign.err"; then
		cat "$tmp/codesign.err" >&2
		die "the signature of $app_name in $tag does not verify. Nothing was installed."
	fi
	mkdir -p "$(dirname "$dest")"
	if [ -e "$dest" ]; then
		rm -rf "$dest.old"
		mv "$dest" "$dest.old"
	fi
	mv "$tmp/unpack/$app_name" "$dest"
	rm -rf "$dest.old"
	if [ -n "$running" ]; then
		# The running PDT keeps the version it started with; open would only bring it forward.
		say "Installed in $dest. The $product that is running keeps its version until you log out and back in."
	else
		say "Installed in $dest. Opening it once so it starts at login..."
		open "$dest"
	fi
}

install_linux() {
	[ -x "$tmp/unpack/$slug/$slug-host" ] || die "$asset does not hold $slug/$slug-host."
	mkdir -p "$(dirname "$dest")"
	# The new folder goes in beside the old one, then takes its place: a running pdt-host keeps its files.
	rm -rf "$dest.new" "$dest.old"
	mv "$tmp/unpack/$slug" "$dest.new"
	if [ -e "$dest" ]; then mv "$dest" "$dest.old"; fi
	mv "$dest.new" "$dest"
	rm -rf "$dest.old"
	say "Installed in $dest. Registering it as a systemd user service..."
	if [ -n "$force" ]; then set -- --replace; else set --; fi
	finish="\"$dest/$slug-host\" install $*"
	# Without a desktop, linger (below) asks about lingering, so pdt-host doesn't say it too.
	if ! desktop; then set -- "$@" --no-linger-hint; fi
	if ! "$dest/$slug-host" install "$@"; then
		die "$product is in $dest, but registering it as a systemd user service failed (above). Once that is fixed, finish the install with: $finish"
	fi
	linger
	if desktop; then
		say "The one-line install has no tray icon. For one, install the AppImage instead: $site/docs/getting-started/install/#install-by-hand"
	fi
}

desktop() {
	[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]
}

# A user service stops at logout; on a machine without a desktop session, offer to keep it running from boot.
linger() {
	if desktop; then return 0; fi
	command -v loginctl >/dev/null 2>&1 || return 0
	user="${USER:-$(id -un)}"
	if loginctl show-user "$user" --property=Linger 2>/dev/null | grep -q '=yes'; then return 0; fi
	answer=""
	if [ -n "$yes" ]; then
		answer=y
	elif (: </dev/tty) 2>/dev/null; then
		printf 'Keep %s running after you log out, and start it at boot (loginctl enable-linger)? [Y/n] ' "$product" >/dev/tty
		read -r answer </dev/tty || answer=n
		answer="${answer:-y}"
	fi
	case "$answer" in
	y | Y | yes | Yes)
		loginctl enable-linger "$user" || say "loginctl enable-linger failed; run it yourself: loginctl enable-linger \"\$USER\""
		;;
	*) say "PDT stops when you log out. To keep it running from boot: loginctl enable-linger \"\$USER\"" ;;
	esac
}

# glibc 2.35 or newer (Ubuntu 22.04 or later), which the Linux builds need.
check_glibc() {
	v="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{ print $2 }')" || v=""
	if [ -z "$v" ]; then
		die "$product for Linux needs glibc 2.35 or newer (Ubuntu 22.04 or later), and this system has no glibc (musl?)."
	fi
	major="${v%%.*}"
	minor="${v#*.}"
	minor="${minor%%.*}"
	if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 35 ]; }; then
		die "$product for Linux needs glibc 2.35 or newer (Ubuntu 22.04 or later); this system has glibc $v."
	fi
}

# GitHub's API: GET $1 into $2. Fails on an HTTP error.
fetch_json() {
	if [ -n "${GITHUB_TOKEN:-}" ]; then
		curl -fsSL -H "Accept: application/vnd.github+json" -H "Authorization: Bearer $GITHUB_TOKEN" "$api$1" -o "$2"
	else
		curl -fsSL -H "Accept: application/vnd.github+json" "$api$1" -o "$2"
	fi
}

# The release file named $1 into $2 ($3: curl's progress option, else silent), from github.com, or through the API
# with a token (a private repository needs that). Sets http to GitHub's answer (000: none; 404 also for a file the
# release's asset list doesn't have). Fails when it isn't the file.
download() {
	progress="${3:--sS}"
	if [ -n "${GITHUB_TOKEN:-}" ]; then
		id="$(find_asset "$1" <"$tmp/assets")" || { http=404; return 1; }
		http="$(curl -fL "$progress" -w '%{http_code}' -H "Accept: application/octet-stream" \
			-H "Authorization: Bearer $GITHUB_TOKEN" "$api/repos/$repo/releases/assets/$id" -o "$2")" || return 1
	else
		http="$(curl -fL "$progress" -w '%{http_code}' "$web/$repo/releases/download/$tag/$1" -o "$2")" || return 1
	fi
}

# Stops on a download of $1 that failed other than with a 404, saying why when GitHub's answer tells.
download_failed() {
	case "$http" in
	000 | "") why="GitHub did not answer: check the connection and try again." ;;
	403 | 429)
		if [ -n "${GITHUB_TOKEN:-}" ]; then
			why="GitHub refused it (HTTP $http): check that GITHUB_TOKEN can read $repo, or wait for its rate limit to reset."
		else
			why="GitHub refused it (HTTP $http): its rate limit may be used up. Try again later, or set GITHUB_TOKEN to a token that can read $repo and run this again."
		fi
		;;
	401) why="GitHub refused GITHUB_TOKEN (HTTP 401): check that it is current and can read $repo." ;;
	*) why="GitHub answered HTTP $http. Try again later." ;;
	esac
	die "could not download $1 of $tag. $why"
}

# The newest stable release's tag, from where github.com's releases/latest redirects (.../releases/tag/<tag>). Fails
# when it doesn't redirect to a tag (no release yet, or no answer).
latest_tag() {
	to="$(curl -fsS -o /dev/null -w '%{redirect_url}' "$web/$repo/releases/latest")" || return 1
	case "$to" in */releases/tag/?*) ;; *) return 1 ;; esac
	t="${to##*/releases/tag/}"
	case "$t" in */* | *\?*) return 1 ;; esac
	printf '%s\n' "$t"
}

# The newest release a host on the nightly channel takes (stdin: the releases, newest first): a nightly build or a
# full release.
nightly_tag() {
	field tag_name | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+(-nightly\.[0-9.]+)?$' | head -n 1
}

no_nightly() {
	die "found no nightly build of $product (the releases: https://github.com/$repo/releases)."
}

# The file names SHA256SUMS lists, one per line.
sum_names() {
	awk '{ f = $2; sub(/^\*/, "", f); if (f != "") print f }' "$tmp/SHA256SUMS"
}

no_releases() {
	if [ -n "${GITHUB_TOKEN:-}" ]; then
		die "could not read $product's releases from GitHub with GITHUB_TOKEN: check that it can read $repo."
	fi
	die "could not read $product's releases from GitHub. Try again later; if GitHub's rate limit is used up or $repo is a private fork, set GITHUB_TOKEN to a token that can read it and run this again."
}

# GitHub's JSON with each field on its own line (a field's value has no comma or brace in what is read here).
split_json() {
	# shellcheck disable=SC2020 # each of the three becomes a newline
	tr ',{}' '\n\n\n'
}

# The values of a JSON string field, one per line, in order (stdin: GitHub's JSON, compact or pretty).
field() {
	split_json | sed -n "s/^[[:space:]]*\"$1\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p"
}

# A release's assets as "<id> <name>" lines (with a token): each asset's API url (…/releases/assets/<id>) comes before its
# browser_download_url (…/<name>).
assets() {
	split_json | awk '
		/^[[:space:]]*"url"[[:space:]]*:[[:space:]]*"[^"]*\/releases\/assets\/[0-9]+"/ {
			id = $0; sub(/.*\/releases\/assets\//, "", id); sub(/".*/, "", id)
		}
		/^[[:space:]]*"browser_download_url"[[:space:]]*:/ {
			name = $0; sub(/"[[:space:]]*$/, "", name); sub(/.*\//, "", name)
			if (id != "") print id, name
			id = ""
		}'
}

# SHA256SUMS.sig is an ECDSA P-256 signature (DER) over the tag, a newline, then SHA256SUMS: it vouches for this
# release only.
verify_signature() {
	{
		echo '-----BEGIN PUBLIC KEY-----'
		printf '%s\n' "$signing_key" | fold -w 64
		echo '-----END PUBLIC KEY-----'
	} >"$tmp/release-key.pem"
	{
		printf '%s\n' "$tag"
		cat "$tmp/SHA256SUMS"
	} >"$tmp/signed"
	openssl dgst -sha256 -verify "$tmp/release-key.pem" -signature "$tmp/SHA256SUMS.sig" "$tmp/signed" >/dev/null 2>&1
}

# The id of the asset named $1 (stdin: assets' lines). Fails when there is none.
find_asset() {
	awk -v n="$1" '$2 == n { print $1; found = 1; exit } END { exit !found }'
}

need() {
	command -v "$1" >/dev/null 2>&1 || die "this needs $1; install it and run this again."
}

say() {
	printf '%s\n' "$1"
}

die() {
	printf 'pdt install: %s\n' "$1" >&2
	exit 1
}

usage() {
	cat <<'EOF'
Installs Personal Dev Team (PDT) on macOS or Linux.

  curl -fsSL https://pdt.dev/install.sh | sh -s -- [options]

  --channel stable|nightly  the newest stable release (default), or the newest nightly build
  --version X.Y.Z           an exact release
  --force                   install again over an existing install
  --yes                     answer yes to questions
  -h, --help                this help

No GitHub token is needed. Set GITHUB_TOKEN only for a private fork, or when GitHub's rate limit is used up.
EOF
}

pdt_install "$@"
