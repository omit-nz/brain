#!/usr/bin/env bash
# Tests for install.sh, with curl stubbed to serve a fake release from a folder and the release's
# brain a stub that answers release-verify, install-dir and --version. Runs on macOS (Apple
# silicon) and Linux (x86_64), the hosts install.sh serves.
#
# Usage: install.test.sh
#
# Each check is a string evaluated after the run, and the tests are called from a list, which
# the linter reads as unused functions (SC2317 or SC2329 by version), unexpanded quotes and an
# unused $status.
# shellcheck disable=SC2016,SC2034,SC2317,SC2329
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
case "$(uname -s) $(uname -m)" in
  "Darwin arm64") target=aarch64-apple-darwin ;;
  "Linux x86_64") target=x86_64-unknown-linux-gnu ;;
  *) echo "install.test.sh: install.sh serves no release for this host" >&2; exit 1 ;;
esac
version=0.2.0
name="brain-$version-$target.tar.gz"
failed=0

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi; }

# A fake release in $box/release, a home with an older install, and stubs on PATH.
setup() {
  box=$(mktemp -d)
  mkdir -p "$box/bin" "$box/home" "$box/tmp" "$box/release" "$box/pack/brain-$version/lib"
  dest="$box/data/bin/brain-current"
  cat >"$box/pack/brain-$version/brain" <<EOF
#!/usr/bin/env bash
case "\$1" in
  release-verify) exit "\${VERIFY_EXIT:-0}" ;;
  install-dir) echo "$dest" ;;
  --version) echo "brain $version (protocol 1)" ;;
esac
EOF
  chmod +x "$box/pack/brain-$version/brain"
  echo "runtime $version" >"$box/pack/brain-$version/lib/libonnxruntime"
  tar -czf "$box/release/$name" -C "$box/pack" "brain-$version"
  (cd "$box/release" && sha "$name" >SHA256SUMS)

  mkdir -p "$dest/lib"
  echo "binary old" >"$dest/brain"
  echo "runtime old" >"$dest/lib/libonnxruntime"

  cat >"$box/bin/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >>"$LOG"
out= write=
while [ $# -gt 1 ]; do
  case "$1" in
    -o) out=$2; shift ;;
    -w) write=$2; shift ;;
  esac
  shift
done
url=$1
if [ -n "$write" ]; then echo "https://github.com/omit-nz/brain/releases/tag/v$LATEST"; exit 0; fi
src="$RELEASE/${url##*/}"
[ -f "$src" ] || exit 22
cp "$src" "$out"
EOF
  cat >"$box/bin/mv" <<'EOF'
#!/usr/bin/env bash
if [ -n "${MV_FAIL_INTO:-}" ] && [ "${!#}" = "$MV_FAIL_INTO" ] && [ "${1##*/}" = brain ]; then
  echo "mv: no space left on device" >&2
  exit 1
fi
exec /bin/mv "$@"
EOF
  chmod +x "$box/bin/"*
  VERIFY_EXIT=0
  MV_FAIL_INTO=
}

run_install() {
  set +e
  PATH="$box/bin:$PATH" HOME="$box/home" TMPDIR="$box/tmp" LOG="$box/curl.log" \
    RELEASE="$box/release" LATEST="$version" VERIFY_EXIT="$VERIFY_EXIT" MV_FAIL_INTO="$MV_FAIL_INTO" \
    bash "$here/install.sh" "$@" >"$box/out" 2>&1
  status=$?
  set -e
}

check() {
  if ! eval "$2"; then
    echo "FAIL $1: $2"
    sed 's/^/  | /' "$box/out"
    failed=1
    this_failed=1
  fi
}

old_install_kept() {
  check "$1" "[ \"\$(cat '$dest/brain')\" = 'binary old' ]"
  check "$1" "[ \"\$(cat '$dest/lib/libonnxruntime')\" = 'runtime old' ]"
}
nothing_left_behind() {
  check "$1" "[ \"\$(ls -A '$box/data/bin')\" = brain-current ]"
  check "$1" "[ -z \"\$(ls -A '$box/tmp')\" ]"
}

installs_a_verified_release() {
  local t=${FUNCNAME[0]}
  setup
  run_install "$version"
  check "$t" '[ "$status" = 0 ]'
  check "$t" "cmp -s '$dest/brain' '$box/pack/brain-$version/brain'"
  check "$t" "[ \"\$(cat '$dest/lib/libonnxruntime')\" = 'runtime $version' ]"
  check "$t" "[ \"\$(readlink '$box/home/.local/bin/brain')\" = '$dest/brain' ]"
  check "$t" "grep -q 'brain $version (protocol 1)' '$box/out'"
  nothing_left_behind "$t"
}

installs_the_newest_when_no_version_is_given() {
  local t=${FUNCNAME[0]}
  setup
  run_install
  check "$t" '[ "$status" = 0 ]'
  check "$t" "cmp -s '$dest/brain' '$box/pack/brain-$version/brain'"
}

a_checksum_mismatch_installs_nothing() {
  local t=${FUNCNAME[0]}
  setup
  echo "0000000000000000000000000000000000000000000000000000000000000000  $name" >"$box/release/SHA256SUMS"
  run_install "$version"
  check "$t" '[ "$status" != 0 ]'
  old_install_kept "$t"
  nothing_left_behind "$t"
}

an_unsigned_archive_installs_nothing() {
  local t=${FUNCNAME[0]}
  setup
  VERIFY_EXIT=1
  run_install "$version"
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'not signed' '$box/out'"
  old_install_kept "$t"
  nothing_left_behind "$t"
}

a_failed_swap_keeps_the_old_install() {
  local t=${FUNCNAME[0]}
  setup
  MV_FAIL_INTO=$dest
  run_install "$version"
  check "$t" '[ "$status" != 0 ]'
  old_install_kept "$t"
  nothing_left_behind "$t"
}

a_version_with_a_path_in_it_is_refused() {
  local t=${FUNCNAME[0]} bad
  for bad in "1.0.0/../x" "1.0.0 x" "1.0.0?x=1" "v"; do
    setup
    run_install "$bad"
    check "$t" '[ "$status" != 0 ]'
    check "$t" "[ ! -e '$box/curl.log' ]"
    old_install_kept "$t"
    rm -rf "$box"
  done
  setup
}

every_download_stays_on_https() {
  local t=${FUNCNAME[0]}
  setup
  run_install
  check "$t" '[ "$status" = 0 ]'
  check "$t" "[ \"\$(grep -c . '$box/curl.log')\" = 3 ]"
  check "$t" "! grep -v -e \"--proto =https\" '$box/curl.log' | grep -q ."
  check "$t" "! grep -v -e \"--proto-redir =https\" '$box/curl.log' | grep -q ."
}

for t in installs_a_verified_release installs_the_newest_when_no_version_is_given \
  a_checksum_mismatch_installs_nothing an_unsigned_archive_installs_nothing \
  a_failed_swap_keeps_the_old_install a_version_with_a_path_in_it_is_refused \
  every_download_stays_on_https; do
  this_failed=0
  "$t"
  [ "$this_failed" = 0 ] && echo "ok $t"
  rm -rf "$box"
done
exit "$failed"
