#!/usr/bin/env bash
# Changelog fragments (docs/SDLC.md "Changelog").
#
#   scripts/changelog.sh check [<fragment>...]   validate fragments (default: every one in .changes/unreleased)
#   scripts/changelog.sh <version>               compile .changes/unreleased into CHANGELOG.md
#   scripts/changelog.sh notes <version>         print <version>'s section of CHANGELOG.md (the release notes)
#   scripts/changelog.sh preview                 print the notes the unreleased fragments would make, changing nothing
#                                                (an experimental release's notes)
#
# A fragment is `.changes/unreleased/<TICKET>.md` (or `<TICKET>-<anything>.md` for a ticket's second note):
#
#   ---
#   kind: added | changed | fixed | breaking | internal
#   ---
#   One or two sentences for people who use PDT (or a short Markdown list).
#
# Compiling adds a `## <version> (<date>)` section at the top of CHANGELOG.md, grouped Breaking, Added, Changed,
# Fixed (internal fragments are left out), prints the same notes without the heading to stdout (the release notes),
# and moves the fragments to `.changes/<version>/`.
#
# Options (before the command): `-C <dir>` works in <dir> instead of the repo root. CHANGELOG_DATE overrides the date
# (YYYY-MM-DD). Bash 3.2 compatible (macOS).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "-C" ]; then root="$2"; shift 2; fi

KINDS="breaking added changed fixed internal"
unreleased="$root/.changes/unreleased"

die() { printf 'changelog: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '3,8p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# The fragment's kind, or nothing when its front matter is not valid.
kind_of() {
  awk 'NR == 1 { if ($0 != "---") exit; next }
       /^---[[:space:]]*$/ { exit }
       /^kind:/ { sub(/^kind:[[:space:]]*/, ""); sub(/[[:space:]]+$/, ""); print; exit }' "$1"
}

# The fragment's text: everything after the closing `---`, without leading or trailing blank lines.
body_of() {
  awk 'NR == 1 { next }
       !closed { if ($0 ~ /^---[[:space:]]*$/) closed = 1; next }
       { lines[++n] = $0 }
       END {
         first = 1; while (first <= n && lines[first] ~ /^[[:space:]]*$/) first++
         last = n; while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
         for (i = first; i <= last; i++) print lines[i]
       }' "$1"
}

# Prints a problem with one fragment, or nothing when it is fine.
problem_with() {
  local f="$1" name kind
  name="$(basename "$f")"
  [ -f "$f" ] || { echo "$f: no such file"; return; }
  case "$name" in
    *.md) ;;
    *) echo "$f: a fragment is a .md file"; return ;;
  esac
  if ! printf '%s' "$name" | grep -qE '^[A-Z][A-Z0-9]*-[0-9]+(-[A-Za-z0-9._-]+)?\.md$'; then
    echo "$f: name it after its ticket, <KEY>-<n>.md (or <KEY>-<n>-<anything>.md for a second one)"; return
  fi
  kind="$(kind_of "$f")"
  if [ -z "$kind" ]; then
    echo "$f: it must start with front matter: a '---' line, 'kind: <kind>', a '---' line"; return
  fi
  case " $KINDS " in
    *" $kind "*) ;;
    *) echo "$f: kind '$kind' is not one of: $KINDS"; return ;;
  esac
  [ -n "$(body_of "$f")" ] || echo "$f: it has no text after its front matter"
}

# The unreleased fragments in ticket order (PDT-9 before PDT-10, PDT-2.md before PDT-2-api.md).
fragments() {
  [ -d "$unreleased" ] || return 0
  (cd "$unreleased" && find . -maxdepth 1 -type f -name '*.md') | sed 's#^\./##; s/\.md$//' \
    | LC_ALL=C sort -t- -k1,1 -k2,2n -k3 | sed "s#^#$unreleased/#; s/\$/.md/"
}

check() {
  local files f p bad=0
  if [ $# -gt 0 ]; then files="$(printf '%s\n' "$@")"; else files="$(fragments)"; fi
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    p="$(problem_with "$f")"
    if [ -n "$p" ]; then echo "$p" >&2; bad=1; fi
  done <<EOF
$files
EOF
  [ "$bad" = 0 ] || die "fix the fragments above (docs/SDLC.md \"Changelog\")"
}

# A fragment as list items: plain text becomes one item; a fragment that is already a list keeps its items.
as_items() {
  local body
  body="$(body_of "$1")"
  case "$body" in
    "- "*|"* "*) printf '%s\n' "$body" ;;
    *) printf '%s\n' "$body" | awk 'NR == 1 { print "- " $0; next } { print ($0 == "" ? "" : "  " $0) }' ;;
  esac
}

# The notes of the fragments given one per line in $1: grouped Breaking, Added, Changed, Fixed, internal ones left out.
section_of() {
  local files="$1" f kind title section="" group
  for kind in breaking added changed fixed; do
    group=""
    while IFS= read -r f; do
      [ "$(kind_of "$f")" = "$kind" ] || continue
      group="$group$(as_items "$f")"$'\n'
    done <<EOF
$files
EOF
    [ -n "$group" ] || continue
    case "$kind" in
      breaking) title="Breaking" ;;
      added) title="Added" ;;
      changed) title="Changed" ;;
      fixed) title="Fixed" ;;
    esac
    [ -z "$section" ] || section="$section"$'\n'
    section="$section### $title"$'\n\n'"$group"
  done
  [ -n "$section" ] || section="Internal changes only."$'\n'
  printf '%s' "$section"
}

compile() {
  local version="$1" date files f section changelog="$root/CHANGELOG.md" dest tmp
  date="${CHANGELOG_DATE:-$(date +%Y-%m-%d)}"
  dest="$root/.changes/$version"
  case "$version" in
    ""|unreleased|*/*|.*) die "'$version' is not a version" ;;
  esac
  files="$(fragments)"
  [ -n "$files" ] || die "no fragments in .changes/unreleased"
  check
  [ ! -e "$dest" ] || die ".changes/$version already exists"
  [ -f "$changelog" ] || die "no CHANGELOG.md in $root"
  # Kept with its trailing newline (a command substitution would drop it).
  section="$(section_of "$files"; echo x)"; section="${section%x}"

  # The new section goes before the first `## ` heading (the newest release), or at the end when there is none.
  tmp="$changelog.tmp"
  # Passed through the environment: awk -v would read backslashes in the notes as escapes.
  HEADING="## $version ($date)" SECTION="$section" awk '
    !done && /^## / { print ENVIRON["HEADING"]; print ""; printf "%s\n", ENVIRON["SECTION"]; done = 1 }
    { print }
    END { if (!done) { print ""; print ENVIRON["HEADING"]; print ""; printf "%s", ENVIRON["SECTION"] } }' \
    "$changelog" > "$tmp"
  mv "$tmp" "$changelog"

  mkdir -p "$dest"
  while IFS= read -r f; do mv "$f" "$dest/"; done <<EOF
$files
EOF
  printf '%s' "$section"
}

# The notes the unreleased fragments would make, as compile prints them, leaving the fragments where they are.
preview() {
  local files
  files="$(fragments)"
  if [ -z "$files" ]; then
    echo "No changes since the last release."
    return
  fi
  check
  section_of "$files"
}

# The notes of a compiled version: its section of CHANGELOG.md without the heading or surrounding blank lines, as
# compile printed them. The release workflow publishes them with the release.
notes() {
  local version="$1" changelog="$root/CHANGELOG.md" out
  [ -f "$changelog" ] || die "no CHANGELOG.md in $root"
  out="$(V="$version" awk '
    /^## / { if (inside) exit; if (index($0, "## " ENVIRON["V"] " (") == 1) { inside = 1; next } }
    inside { lines[++n] = $0 }
    END {
      first = 1; while (first <= n && lines[first] ~ /^[[:space:]]*$/) first++
      last = n; while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
      for (i = first; i <= last; i++) print lines[i]
    }' "$changelog")"
  [ -n "$out" ] || die "CHANGELOG.md has no section for $version"
  printf '%s\n' "$out"
}

case "${1:-}" in
  ""|-h|--help) usage ;;
  check) shift; check "$@" ;;
  notes) [ $# -eq 2 ] || usage; notes "$2" ;;
  preview) [ $# -eq 1 ] || usage; preview ;;
  -*) usage ;;
  *) [ $# -eq 1 ] || usage; compile "$1" ;;
esac
