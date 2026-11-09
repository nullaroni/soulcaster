#!/bin/bash
# Tests install.sh's logic. Nothing here installs anything: every tool that could reach the network,
# run an app or copy one is a stub, PATH holds only those stubs and a few harmless real tools, and
# the destination is a sandbox directory. The script's last line, the only one that would use
# /Applications, is removed before anything runs, and the truncation test stops short of it.
# Usage: /bin/bash packaging/test-install.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
installer="$root/install.sh"
built_app="$root/target/package/MacFlayer.app"

sandbox=$(mktemp -d)
trap 'chmod -R u+w "$sandbox"; rm -rf "$sandbox"' EXIT

passed=0
failed=0
current=""

begin() {
  current="$1"
  case_dir=$(mktemp -d "$sandbox/case.XXXXXX")
  mkdir "$case_dir/tmp" "$case_dir/home" "$case_dir/release" "$case_dir/Applications"
  dest="$case_dir/Applications"
  team=TESTTEAM01
  log="$case_dir/calls.log"
  : >"$log"
  case_failed=0
  publish_release
}

not_ok() {
  case_failed=1
  echo "FAIL  $current: $1"
  echo "      output: ${output:-}"
  echo "      calls: $(tr '\n' ';' <"$log")"
}

# --- Stubs -----------------------------------------------------------------------------------------

bin="$sandbox/bin"
mkdir "$bin"

write_stub() {
  {
    # shellcheck disable=SC2016
    printf '#!/bin/bash\nprintf "%%s\\n" "%s $*" >>"$STUB_LOG"\n' "$1"
    cat
  } >"$bin/$1"
  chmod +x "$bin/$1"
}

write_stub curl <<'STUB'
output="" url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    https://*) url="$1"; shift ;;
    *) shift ;;
  esac
done
if [ -n "${STUB_CURL_EXIT:-}" ]; then exit "$STUB_CURL_EXIT"; fi
if [ -f "$STUB_RELEASE/${url##*/}" ]; then /bin/cp "$STUB_RELEASE/${url##*/}" "$output"; else exit 22; fi
STUB

write_stub ditto <<'STUB'
if [ "$1" = "-x" ]; then
  case "${STUB_UNPACK:-app}" in
    fail) exit 1 ;;
    empty) /bin/mkdir -p "$4" ;;
    symlink) /bin/mkdir -p "$4"; /bin/ln -s /Applications/Safari.app "$4/MacFlayer.app" ;;
    app) /bin/mkdir -p "$4/MacFlayer.app/Contents"; echo fake >"$4/MacFlayer.app/Contents/Info.plist" ;;
  esac
else
  /bin/mkdir -p "$2"
  if [ "${STUB_COPY:-ok}" = "fail" ]; then exit 1; fi
  /bin/cp -R "$1/" "$2"
fi
STUB

write_stub codesign <<'STUB'
if [ "${STUB_CODESIGN:-ok}" != "ok" ]; then exit 3; fi
case "$*" in
  "--verify --deep --strict -R=anchor apple generic and certificate leaf[subject.OU] = \"TESTTEAM01\" "*) exit 0 ;;
  *) exit 3 ;;
esac
STUB

write_stub spctl <<'STUB'
printf '%s\n' "${STUB_SPCTL_OUTPUT:-accepted
source=Notarized Developer ID}"
exit "${STUB_SPCTL_EXIT:-0}"
STUB

# macOS's mktemp ignores TMPDIR when given only -d. This keeps the test's copy inside the sandbox.
write_stub mktemp <<'STUB'
exec /usr/bin/mktemp -d "$TMPDIR/tmp.XXXXXX"
STUB

write_stub open <<'STUB'
exit 0
STUB

write_stub uname <<'STUB'
echo "${STUB_UNAME:-Darwin}"
STUB

write_stub sysctl <<'STUB'
echo "${STUB_ARM64:-1}"
STUB

write_stub sw_vers <<'STUB'
echo "${STUB_MACOS:-27.2}"
STUB

write_stub id <<'STUB'
echo "${STUB_UID:-501}"
STUB

# Each of these either needs Command Line Tools or would undo a guarantee of the installer.
for tool in git xcrun xcode-select python3 make clang cc gcc otool lipo stapler softwareupdate sudo osascript xattr; do
  write_stub "$tool" <<'STUB'
exit 127
STUB
done

# The real tools the installer may use besides the stubs above.
real_tools=(rm mv shasum)
for tool in "${real_tools[@]}"; do
  ln -s "$(PATH=/usr/bin:/bin command -v "$tool")" "$bin/$tool"
done

# --- Harness ---------------------------------------------------------------------------------------

if [ "$(tail -n 1 "$installer")" != "macflayer_main" ]; then
  echo "install.sh must end with the line macflayer_main" >&2
  exit 1
fi
lib="$sandbox/install-lib.sh"
sed '$d' "$installer" >"$lib"

publish_release() {
  printf 'fake archive' >"$case_dir/release/MacFlayer-macos-arm64.zip"
  (cd "$case_dir/release" && /usr/bin/shasum -a 256 MacFlayer-macos-arm64.zip >MacFlayer-macos-arm64.zip.sha256)
}

# Runs the installer's real function against the sandbox. Arguments are STUB_* settings.
run_installer() {
  status=0
  # shellcheck disable=SC2016
  output=$(/usr/bin/env -i PATH="$bin" HOME="$case_dir/home" TMPDIR="$case_dir/tmp" \
    STUB_LOG="$log" STUB_RELEASE="$case_dir/release" "$@" \
    /bin/bash -c 'source "$1"; macflayer_install "$2" "$3"' _ "$lib" "$dest" "$team" 2>&1) || status=$?
}

expect_status() {
  if [ "$status" -ne "$1" ]; then not_ok "exit $status, expected $1"; fi
}

expect_output() {
  case "$output" in
    *"$1"*) ;;
    *) not_ok "output lacks '$1'" ;;
  esac
}

expect_no_call() {
  if /usr/bin/grep -qE "^($1)( |\$)" "$log"; then not_ok "called $1"; fi
}

expect_clean() {
  if [ -n "$(find "$case_dir/tmp" -mindepth 1 -maxdepth 1)" ]; then not_ok "temp directory left behind"; fi
  if [ -n "$(find "$dest" -mindepth 1 -maxdepth 1 -name '.MacFlayer*')" ]; then not_ok "staged copy left behind"; fi
}

expect_nothing_installed() {
  expect_clean
  if [ -e "$dest/MacFlayer.app" ]; then not_ok "app was installed"; fi
  expect_no_call open
}

expect_refused_before_download() {
  expect_status 1
  expect_no_call curl
  expect_nothing_installed
}

finish_case() {
  expect_no_call 'git|xcrun|xcode-select|python3|make|clang|cc|gcc|otool|lipo|stapler|softwareupdate|sudo|osascript|xattr'
  if [ "$case_failed" -eq 0 ]; then
    passed=$((passed + 1))
    echo "ok    $current"
  else
    failed=$((failed + 1))
  fi
}

# --- The path that installs ------------------------------------------------------------------------

begin "installs a verified release, in the right order"
run_installer
expect_status 0
if [ ! -f "$dest/MacFlayer.app/Contents/Info.plist" ]; then not_ok "app not in the destination"; fi
order=$(sed -nE 's/^(curl|ditto|codesign|spctl|open) .*/\1/p' "$log" | tr '\n' ' ')
if [ "$order" != "curl curl ditto codesign spctl ditto open " ]; then not_ok "order was: $order"; fi
/usr/bin/grep -q "^open $dest/MacFlayer.app\$" "$log" || not_ok "didn't open the installed app"
/usr/bin/grep -q "^curl --proto =https --tlsv1.2 .*https://github.com/nullaroni/soulcaster/releases/latest/download/MacFlayer-macos-arm64.zip\$" "$log" ||
  not_ok "wrong download command"
/usr/bin/grep -q -- "--output $case_dir/tmp/tmp\.[^/]*/MacFlayer-macos-arm64.zip" "$log" ||
  not_ok "didn't download into a fresh temporary directory"
expect_clean
finish_case

for version in 15.0 15.7.1 26.1 27.2; do
  begin "accepts macOS $version"
  run_installer STUB_MACOS="$version"
  expect_status 0
  finish_case
done

# --- Refusing the machine --------------------------------------------------------------------------

begin "refuses anything but macOS"
run_installer STUB_UNAME=Linux
expect_refused_before_download
expect_output "only on macOS"
finish_case

begin "refuses an Intel Mac"
run_installer STUB_ARM64=0
expect_refused_before_download
expect_output "only on Apple Silicon"
finish_case

begin "refuses macOS 14"
run_installer STUB_MACOS=14.7.1
expect_refused_before_download
expect_output "macOS 15 or later"
finish_case

begin "refuses an unreadable macOS version"
run_installer STUB_MACOS=unknown
expect_refused_before_download
finish_case

begin "refuses to run as root"
run_installer STUB_UID=0
expect_refused_before_download
expect_output "not as root"
finish_case

begin "refuses a destination it can't write to, without asking for a password"
chmod 555 "$dest"
run_installer
expect_refused_before_download
expect_output "never asks for a password"
chmod 755 "$dest"
finish_case

# --- No notarized release --------------------------------------------------------------------------

begin "with no team set, stops before the network and points to the source build"
team=""
run_installer
expect_refused_before_download
expect_output "No notarized release of MacFlayer exists yet"
expect_output "packaging/build-app.sh"
finish_case

begin "with no release published, stops and points to the source build"
rm "$case_dir"/release/*
run_installer
expect_status 1
expect_nothing_installed
expect_no_call ditto
expect_output "No notarized release is available"
expect_output "packaging/build-app.sh"
finish_case

begin "a missing checksum file stops it"
rm "$case_dir"/release/*.sha256
run_installer
expect_status 1
expect_nothing_installed
expect_no_call ditto
finish_case

begin "a network failure is reported as one"
run_installer STUB_CURL_EXIT=6
expect_status 1
expect_nothing_installed
expect_output "curl exit 6"
finish_case

# --- Refusing the download -------------------------------------------------------------------------

begin "refuses a checksum mismatch before unpacking"
printf '%064d  MacFlayer-macos-arm64.zip\n' 0 >"$case_dir/release/MacFlayer-macos-arm64.zip.sha256"
run_installer
expect_status 1
expect_nothing_installed
expect_no_call ditto
expect_output "doesn't match"
finish_case

begin "refuses a malformed checksum file"
echo '<html>not a checksum</html>' >"$case_dir/release/MacFlayer-macos-arm64.zip.sha256"
run_installer
expect_status 1
expect_nothing_installed
expect_no_call ditto
finish_case

begin "refuses an empty checksum file"
: >"$case_dir/release/MacFlayer-macos-arm64.zip.sha256"
run_installer
expect_status 1
expect_nothing_installed
expect_no_call ditto
finish_case

begin "refuses an archive that won't unpack"
run_installer STUB_UNPACK=fail
expect_status 1
expect_nothing_installed
expect_no_call codesign
finish_case

begin "refuses an archive without the app"
run_installer STUB_UNPACK=empty
expect_status 1
expect_nothing_installed
expect_no_call codesign
finish_case

begin "refuses an archive whose app is a symlink"
run_installer STUB_UNPACK=symlink
expect_status 1
expect_nothing_installed
expect_no_call codesign
finish_case

# --- Refusing an unverified app --------------------------------------------------------------------

begin "refuses a broken signature"
run_installer STUB_CODESIGN=broken
expect_status 1
expect_nothing_installed
expect_output "failed verification"
finish_case

begin "refuses another team's signature"
team=OTHERTEAM2
run_installer
expect_status 1
expect_nothing_installed
expect_output "OTHERTEAM2"
finish_case

begin "refuses what Gatekeeper rejects"
run_installer STUB_SPCTL_EXIT=3 STUB_SPCTL_OUTPUT="rejected"
expect_status 1
expect_nothing_installed
expect_output "Gatekeeper rejected"
finish_case

begin "refuses a signed but unnotarized app"
run_installer STUB_SPCTL_OUTPUT="accepted
source=Developer ID"
expect_status 1
expect_nothing_installed
expect_output "as notarized"
finish_case

begin "isn't fooled by Gatekeeper being turned off"
run_installer STUB_SPCTL_OUTPUT="accepted
override=security disabled"
expect_status 1
expect_nothing_installed
finish_case

begin "cleans up a failed copy"
run_installer STUB_COPY=fail
expect_status 1
expect_nothing_installed
finish_case

# --- Reruns ----------------------------------------------------------------------------------------

begin "a rerun opens a verified app and downloads nothing"
mkdir "$dest/MacFlayer.app"
echo original >"$dest/MacFlayer.app/marker"
run_installer
expect_status 0
expect_no_call curl
/usr/bin/grep -q "^open $dest/MacFlayer.app\$" "$log" || not_ok "didn't open it"
[ "$(cat "$dest/MacFlayer.app/marker")" = original ] || not_ok "changed the existing app"
finish_case

begin "a rerun refuses an existing app that doesn't verify, and leaves it alone"
mkdir "$dest/MacFlayer.app"
echo original >"$dest/MacFlayer.app/marker"
run_installer STUB_CODESIGN=broken
expect_status 1
expect_no_call 'curl|open'
[ "$(cat "$dest/MacFlayer.app/marker")" = original ] || not_ok "changed the existing app"
finish_case

# --- The script as shipped -------------------------------------------------------------------------

begin "a truncated script does nothing, however it's run"
size=$(wc -c <"$installer" | tr -d ' ')
# The last line is "macflayer_main\n". Stop before it is complete, so the real entry point never runs.
last=$((size - 2))
offsets=""
for ((n = 1; n <= last; n += 173)); do offsets="$offsets $n"; done
for ((n = last - 20; n <= last; n++)); do offsets="$offsets $n"; done
for n in $offsets; do
  partial=$(head -c "$n" "$installer")
  /usr/bin/env -i PATH="$bin" STUB_LOG="$log" /bin/bash -c "$partial" >/dev/null 2>&1 || true
  printf '%s' "$partial" | /usr/bin/env -i PATH="$bin" STUB_LOG="$log" /bin/bash >/dev/null 2>&1 || true
done
if [ -s "$log" ]; then not_ok "a truncated script ran something"; fi
finish_case

code=$(sed -e 's/^[[:space:]]*#.*//' "$installer")

begin "the script parses under /bin/bash and has no eval, password or bypass path"
/bin/bash -n "$installer" || not_ok "syntax error"
for word in eval sudo xattr getopts '"\$@"'; do
  if /usr/bin/grep -qE "$word" <<<"$code"; then not_ok "uses $word"; fi
done
finish_case

begin "the real entry point installs to /Applications and takes no arguments"
# shellcheck disable=SC2016
/usr/bin/grep -q '^  macflayer_install /Applications "\$team_id"$' <<<"$code" ||
  not_ok "macflayer_main doesn't name /Applications"
finish_case

begin "the installer's minimum macOS matches the app's"
major=$(plutil -extract LSMinimumSystemVersion raw "$root/packaging/Info.plist")
major=${major%%.*}
/usr/bin/grep -q "\"\$macos_major\" -ge $major \]" "$installer" || not_ok "not macOS $major"
finish_case

begin "every tool the installer uses is real on a bare Mac, not a Command Line Tools shim"
is_shim() {
  local signature
  signature=$(codesign -dv "$(PATH=/usr/bin:/bin:/usr/sbin:/sbin command -v "$1")" 2>&1 || true)
  case "$signature" in
    *xcode_select*) return 0 ;;
    *) return 1 ;;
  esac
}
is_shim git || not_ok "the shim check doesn't recognize /usr/bin/git"
for tool in curl ditto codesign spctl open uname sysctl sw_vers id mktemp "${real_tools[@]}"; do
  if is_shim "$tool"; then not_ok "$tool is a shim"; fi
done
finish_case

# Uses the real codesign and spctl, which only read the app.
if [ -d "$built_app" ]; then
  begin "the real codesign and spctl refuse a local ad-hoc build"
  # shellcheck disable=SC2016
  if /bin/bash -c 'source "$1"; macflayer_verify_app "$2" TESTTEAM01' _ "$lib" "$built_app" 2>/dev/null; then
    not_ok "an ad-hoc build verified"
  fi
  finish_case
fi

echo
echo "$passed passed, $failed failed"
[ "$failed" -eq 0 ]
