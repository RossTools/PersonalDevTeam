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
# It downloads the release for this system from GitHub, checks it against the release's SHA256SUMS and installs it:
# macOS into ~/Applications (then opens it once, which starts it at login), Linux into ~/.local/opt/pdt as a systemd
# user service. Windows has install.ps1.
#
# Everything runs from pdt_install, called on the last line: a download cut short runs nothing.
#
# For tests only: PDT_INSTALL_API (GitHub's API, https://api.github.com), PDT_INSTALL_LOCAL (where a running PDT
# answers, http://127.0.0.1:7420) and PDT_INSTALL_SYSTEM_APPS (/Applications).

set -eu

pdt_install() {
	repo="RossTools/PersonalDevTeam"
	product="Personal Dev Team"
	slug="pdt"
	port=7420
	site="https://pdt.dev"
	api="${PDT_INSTALL_API:-https://api.github.com}"
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

	if [ "$os" = mac ]; then
		app_name="$product.app"
		dest="$HOME/Applications/$app_name"
		existing=""
		for d in "$HOME/Applications/$app_name" "$sys_apps/$app_name"; do
			if [ -d "$d" ]; then existing="$d"; break; fi
		done
	else
		dest="$HOME/.local/opt/$slug"
		data="${XDG_DATA_HOME:-$HOME/.local/share}/$slug"
		existing=""
		for d in "$dest" "$data/app"; do
			if [ -e "$d/$slug-host" ]; then existing="$d"; break; fi
		done
	fi
	running=""
	if curl -fsS --max-time 3 "$local_url/api/health" 2>/dev/null | grep -q "\"$product\""; then running=1; fi
	if [ -z "$force" ] && { [ -n "$existing" ] || [ -n "$running" ]; }; then
		if [ -n "$existing" ]; then say "$product is already installed in $existing."; else say "$product is already running at $local_url."; fi
		say "It keeps itself up to date: Settings > This host > Updates shows its version and checks for a new one."
		say "To install it again anyway, run this with --force."
		return 0
	fi
	# --force replaces the app where it is; one in a folder only an admin can change (/Applications, usually) is left
	# to the user rather than asking for sudo.
	if [ "$os" = mac ] && [ -n "$existing" ]; then
		dest="$existing"
		[ -w "$(dirname "$dest")" ] ||
			die "$product is installed in $dest, which needs admin rights to replace. It updates itself (Settings > This host > Updates); to reinstall, move it to the Bin first."
	fi

	tmp="$(mktemp -d)"
	# Removes only the folder made above.
	trap 'rm -rf "$tmp"' EXIT
	trap 'exit 130' INT TERM

	say "Finding the release..."
	if [ -n "$version" ]; then
		fetch_json "/repos/$repo/releases/tags/v$version" "$tmp/release.json" ||
			die "there is no $product release v$version (the releases: https://github.com/$repo/releases)."
	elif [ "$channel" = nightly ]; then
		fetch_json "/repos/$repo/releases?per_page=30" "$tmp/releases.json" || no_releases
		# The newest release a host on the nightly channel takes: a nightly build or a full release, listed newest first.
		tag="$(field tag_name <"$tmp/releases.json" | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+(-nightly\.[0-9.]+)?$' | head -n 1)" ||
			true
		[ -n "$tag" ] || die "found no nightly build of $product (the releases: https://github.com/$repo/releases)."
		fetch_json "/repos/$repo/releases/tags/$tag" "$tmp/release.json" || no_releases
	else
		fetch_json "/repos/$repo/releases/latest" "$tmp/release.json" || no_releases
	fi
	tag="$(field tag_name <"$tmp/release.json" | head -n 1)"
	[ -n "$tag" ] || die "GitHub's answer about the release could not be read."
	assets <"$tmp/release.json" >"$tmp/assets"
	if [ "$os" = mac ]; then
		asset="$(cut -d' ' -f2 "$tmp/assets" | grep -E '.\.app\.tar\.gz$' | head -n 1)" || true
	else
		asset="$slug-linux-$arch.tar.gz"
	fi
	asset_id=""
	if [ -n "$asset" ]; then asset_id="$(find_asset "$asset" <"$tmp/assets")" || true; fi
	[ -n "$asset_id" ] || die "release $tag has no download for this system${asset:+ ($asset)}."
	sums_id="$(find_asset SHA256SUMS <"$tmp/assets")" ||
		die "release $tag has no SHA256SUMS, so the download can't be checked. Nothing was installed."

	say "Downloading $product $tag ($asset)..."
	download "$sums_id" "$tmp/SHA256SUMS" || die "could not download SHA256SUMS of $tag."
	download "$asset_id" "$tmp/$asset" || die "could not download $asset."
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
	if [ -n "$force" ]; then
		"$dest/$slug-host" install --replace
	else
		"$dest/$slug-host" install
	fi
	linger
}

# A user service stops at logout; on a machine without a desktop session, offer to keep it running from boot.
linger() {
	if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then return 0; fi
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

# A release asset by id into $2 (through the API, so a token works for a private repository).
download() {
	if [ -n "${GITHUB_TOKEN:-}" ]; then
		curl -fSL --progress-bar -H "Accept: application/octet-stream" -H "Authorization: Bearer $GITHUB_TOKEN" \
			"$api/repos/$repo/releases/assets/$1" -o "$2"
	else
		curl -fSL --progress-bar -H "Accept: application/octet-stream" "$api/repos/$repo/releases/assets/$1" -o "$2"
	fi
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

# A release's assets as "<id> <name>" lines: each asset's API url (…/releases/assets/<id>) comes before its
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
