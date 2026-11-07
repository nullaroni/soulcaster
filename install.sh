#!/bin/bash
# install.sh - Downloads the notarized Soulcaster app, verifies it, copies it to /Applications and
# opens it.
#
#   /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/nullaroni/soulcaster/main/install.sh)"
#
# It uses only tools that ship with macOS, so it works with no Git and no Command Line Tools and
# never triggers their install prompt. It never asks for a password. An app that fails a check is
# refused, and no flag or setting skips a check.
#
# Everything is in functions and only the last line runs them, so a truncated download does
# nothing. /bin/bash on a Mac is 3.2, so nothing newer is used here.

soulcaster_die() {
  printf '\nSoulcaster: %s\n' "$*" >&2
  exit 1
}

soulcaster_build_from_source_steps() {
  printf '%s\n' \
    "To build it from source instead (this needs Git, Rust and Command Line Tools):" \
    "" \
    "  git clone https://github.com/nullaroni/soulcaster.git" \
    "  cd soulcaster" \
    "  packaging/build-app.sh" \
    "  open target/package/Soulcaster.app"
}

soulcaster_die_no_release() {
  soulcaster_die "$1 Stopping, and nothing was changed.

$(soulcaster_build_from_source_steps)"
}

soulcaster_check_machine() {
  local macos_version macos_major

  [ "$(uname -s)" = "Darwin" ] || soulcaster_die "Soulcaster runs only on macOS."
  # Not `uname -m`, which says x86_64 in a Terminal running under Rosetta on Apple Silicon.
  [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = "1" ] ||
    soulcaster_die "Soulcaster runs only on Apple Silicon Macs."

  macos_version=$(sw_vers -productVersion)
  macos_major=${macos_version%%.*}
  case "$macos_major" in
    '' | *[!0-9]*) soulcaster_die "Couldn't read the macOS version from '$macos_version'." ;;
  esac
  [ "$macos_major" -ge 15 ] ||
    soulcaster_die "Soulcaster needs macOS 15 or later, and this Mac has $macos_version. Update macOS, then run this again."

  [ "$(id -u)" != "0" ] || soulcaster_die "Run this as yourself, not as root. It needs no password."
}

# Checks both that the signature is intact and from the expected team, and that Gatekeeper accepts
# it as notarized. curl doesn't quarantine what it downloads, so nothing else would make this check.
# Gatekeeper reports "accepted" when it has been turned off, so the source line matters too.
soulcaster_verify_app() {
  local app="$1" team_id="$2" assessment

  if ! codesign --verify --deep --strict \
    -R="anchor apple generic and certificate leaf[subject.OU] = \"$team_id\"" "$app" 2>/dev/null; then
    echo "Its signature is broken, or isn't from Apple team $team_id." >&2
    return 1
  fi
  if ! assessment=$(spctl -a -vv -t exec "$app" 2>&1); then
    echo "Gatekeeper rejected it." >&2
    return 1
  fi
  case "$assessment" in
    *"source=Notarized Developer ID"*) return 0 ;;
    *)
      echo "Gatekeeper doesn't report it as notarized." >&2
      return 1
      ;;
  esac
}

soulcaster_download() {
  local url="$1" path="$2" status=0

  curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error --output "$path" "$url" ||
    status=$?
  case "$status" in
    0) ;;
    22) soulcaster_die_no_release "No notarized release is available at $url." ;;
    *) soulcaster_die "Couldn't download $url (curl exit $status). Check your connection and run this again." ;;
  esac
}

soulcaster_verify_checksum() {
  local archive="$1" checksum_file="$2" expected="" actual

  read -r expected _ <"$checksum_file" || true
  case "$expected" in
    '' | *[!0-9a-f]*) soulcaster_die "The release's checksum file isn't valid. Nothing was installed." ;;
  esac
  actual=$(shasum -a 256 "$archive")
  actual=${actual%% *}
  [ "$actual" = "$expected" ] ||
    soulcaster_die "The download doesn't match the release's checksum. Nothing was installed."
}

# applications_dir and team_id are parameters so that tests can call this with a sandbox and a fake
# team. The real values are set by soulcaster_main, and nothing in the environment can change them.
soulcaster_install() {
  set -euo pipefail

  local applications_dir="$1" team_id="$2"
  local release_url="https://github.com/nullaroni/soulcaster/releases/latest/download"
  local archive_name="Soulcaster-macos-arm64.zip"
  local installed="$applications_dir/Soulcaster.app"
  local archive extracted

  soulcaster_check_machine

  [ -n "$team_id" ] || soulcaster_die_no_release "No notarized release of Soulcaster exists yet, so there is nothing this installer can verify."

  # Replacing an app could run into macOS's App Management protection, so a rerun only opens it.
  if [ -e "$installed" ] || [ -L "$installed" ]; then
    soulcaster_verify_app "$installed" "$team_id" ||
      soulcaster_die "$installed exists but doesn't verify, so it wasn't opened or changed. Move it to the Trash, then run this again."
    echo "Soulcaster is already installed. Opening it. To get a newer release, move it to the Trash and run this again."
    open "$installed"
    return 0
  fi

  [ -w "$applications_dir" ] ||
    soulcaster_die "Your account can't write to $applications_dir, and this installer never asks for a password. Sign in as an administrator, then run this again."

  # Not local: the exit trap runs after this function's variables are gone.
  soulcaster_work_dir=$(mktemp -d) || soulcaster_die "Couldn't create a temporary directory."
  soulcaster_staged=""
  trap 'rm -rf "$soulcaster_work_dir"; if [ -n "$soulcaster_staged" ]; then rm -rf "$soulcaster_staged"; fi' EXIT

  archive="$soulcaster_work_dir/$archive_name"
  extracted="$soulcaster_work_dir/extracted"

  echo "Downloading Soulcaster..."
  soulcaster_download "$release_url/$archive_name" "$archive"
  soulcaster_download "$release_url/$archive_name.sha256" "$archive.sha256"
  soulcaster_verify_checksum "$archive" "$archive.sha256"

  echo "Verifying the signature and notarization..."
  ditto -x -k "$archive" "$extracted" ||
    soulcaster_die "The download couldn't be unpacked. Nothing was installed."
  if [ ! -d "$extracted/Soulcaster.app" ] || [ -L "$extracted/Soulcaster.app" ]; then
    soulcaster_die "The download doesn't contain Soulcaster.app. Nothing was installed."
  fi
  soulcaster_verify_app "$extracted/Soulcaster.app" "$team_id" ||
    soulcaster_die "This app failed verification, so it was refused. Nothing was installed."

  # Copied beside its final name and then renamed, so an interrupted copy never looks installed.
  echo "Copying it to $applications_dir..."
  soulcaster_staged="$applications_dir/.Soulcaster.app.installing.$$"
  ditto "$extracted/Soulcaster.app" "$soulcaster_staged" ||
    soulcaster_die "Couldn't copy the app to $applications_dir. Nothing was installed."
  mv "$soulcaster_staged" "$installed" ||
    soulcaster_die "Couldn't move the app into place in $applications_dir. Nothing was installed."
  soulcaster_staged=""

  echo "Opening Soulcaster."
  open "$installed"
}

soulcaster_main() {
  # The Apple Team ID on the owner's Developer ID certificate. Empty until the owner enrolls: no
  # notarized release can exist before then, so the installer stops. See docs/releasing.md.
  local team_id=""

  soulcaster_install /Applications "$team_id"
}

soulcaster_main
