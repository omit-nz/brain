#!/usr/bin/env bash
# Tests for release-key.sh, with zipsign, op and gh stubbed on PATH. Each stub records its argv
# and its stdin, so a test can say where the private key went. Runs on Linux and macOS.
#
# Usage: release-key.test.sh
#
# Each check is a string evaluated after the run, and the tests are called from a list, which
# which the linter reads as unused functions, unexpanded quotes and an unused $status.
# shellcheck disable=SC2016,SC2034,SC2329
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
failed=0

# A sandbox repo holding a copy of the script and the placeholder key, plus the stubs.
setup() {
  box=$(mktemp -d)
  mkdir -p "$box/repo/scripts" "$box/repo/crates/brain/src/keys" "$box/bin" "$box/log" "$box/tmp"
  cp "$here/release-key.sh" "$box/repo/scripts/"
  head -c 32 /dev/zero >"$box/repo/crates/brain/src/keys/release.pub"

  cat >"$box/bin/zipsign" <<'EOF'
#!/usr/bin/env bash
echo "zipsign $*" >>"$LOG/argv"
n=$(cat "$LOG/keys" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" >"$LOG/keys"
printf 'SENTINEL-PRIVATE-KEY-%02d-%040d' "$n" 0 >"$2"
printf 'PUBLIC-KEY-%02d-%018d' "$n" 0 >"$3"
EOF

  cat >"$box/bin/op" <<'EOF'
#!/usr/bin/env bash
echo "op $*" >>"$LOG/argv"
n=$(ls "$LOG" | grep -c '^op-stdin' || true)
if [ "$1" = read ]; then
  : >"$LOG/op-stdin.$n"
  printf '%s' "$OLD_KEY_B64"
elif [ "$1 $2" = "item create" ]; then
  cat >"$LOG/op-stdin.$n"
  # op prints the item it made, values and all.
  echo "{\"created\": $(cat "$LOG/op-stdin.$n")}"
else
  : >"$LOG/op-stdin.$n"
fi
EOF

  cat >"$box/bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh $*" >>"$LOG/argv"
n=$(ls "$LOG" | grep -c '^gh-stdin' || true)
cat >"$LOG/gh-stdin.$n"
echo "Set Actions secret $3 for omit-nz/brain"
EOF
  chmod +x "$box/bin/"*
  export LOG="$box/log"
}

run_script() {
  set +e
  PATH="$box/bin:$PATH" TMPDIR="$box/tmp" OLD_KEY_B64="$OLD_KEY_B64" \
    "$box/repo/scripts/release-key.sh" "$@" >"$box/out" 2>&1
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

# Neither the key nor its base64 form appears in any argv or in the script's own output.
never_printed() {
  local name=$1 key=$2 b64
  b64=$(printf '%s' "$key" | base64 | tr -d '\n')
  check "$name" "! grep -qF -e '$key' -e '$b64' '$LOG/argv'"
  check "$name" "! grep -qF -e '$key' -e '$b64' '$box/out'"
}

key_n() { printf 'SENTINEL-PRIVATE-KEY-%02d-%040d' "$1" 0; }

the_key_script_never_prints_the_key() {
  setup
  OLD_KEY_B64=
  run_script init
  local key b64
  key=$(key_n 1)
  b64=$(printf '%s' "$key" | base64 | tr -d '\n')
  check "${FUNCNAME[0]}" '[ "$status" = 0 ]'
  never_printed "${FUNCNAME[0]}" "$key"
  check "${FUNCNAME[0]}" "grep -qF '$b64' '$LOG/op-stdin.0'"
  check "${FUNCNAME[0]}" "[ \"\$(cat '$LOG/gh-stdin.0')\" = '$b64' ]"
  check "${FUNCNAME[0]}" "grep -q 'gh secret set RELEASE_SIGNING_KEY --repo omit-nz/brain' '$LOG/argv'"
  check "${FUNCNAME[0]}" "[ \"\$(cat '$box/repo/crates/brain/src/keys/release.pub')\" = \"\$(printf 'PUBLIC-KEY-%02d-%018d' 1 0)\" ]"
  check "${FUNCNAME[0]}" "[ -z \"\$(ls -A '$box/tmp')\" ]"
}

init_refuses_when_a_key_exists() {
  setup
  OLD_KEY_B64=
  printf 'PUBLIC-KEY-07-%018d' 0 >"$box/repo/crates/brain/src/keys/release.pub"
  run_script init
  check "${FUNCNAME[0]}" '[ "$status" != 0 ]'
  check "${FUNCNAME[0]}" "grep -q 'rotate' '$box/out'"
  check "${FUNCNAME[0]}" "[ ! -e '$LOG/argv' ]"
}

rotate_keeps_the_old_key_for_one_release() {
  setup
  local old new old_b64 new_b64
  old=$(key_n 0)
  old_b64=$(printf '%s' "$old" | base64 | tr -d '\n')
  OLD_KEY_B64=$old_b64
  printf 'PUBLIC-KEY-00-%018d' 0 >"$box/repo/crates/brain/src/keys/release.pub"
  run_script rotate
  new=$(key_n 1)
  new_b64=$(printf '%s' "$new" | base64 | tr -d '\n')
  check "${FUNCNAME[0]}" '[ "$status" = 0 ]'
  never_printed "${FUNCNAME[0]}" "$old"
  never_printed "${FUNCNAME[0]}" "$new"
  check "${FUNCNAME[0]}" "grep -q 'gh secret set RELEASE_SIGNING_KEY_PREVIOUS --repo omit-nz/brain' '$LOG/argv'"
  check "${FUNCNAME[0]}" "grep -qxF '$old_b64' '$LOG/gh-stdin.0'"
  check "${FUNCNAME[0]}" "grep -qxF '$new_b64' '$LOG/gh-stdin.1'"
  check "${FUNCNAME[0]}" "grep -q 'op item edit' '$LOG/argv'"
  check "${FUNCNAME[0]}" "[ \"\$(cat '$box/repo/crates/brain/src/keys/release.pub')\" = \"\$(printf 'PUBLIC-KEY-%02d-%018d' 1 0)\" ]"
}

rotate_without_a_key_is_refused() {
  setup
  OLD_KEY_B64=
  run_script rotate
  check "${FUNCNAME[0]}" '[ "$status" != 0 ]'
  check "${FUNCNAME[0]}" "grep -q 'init' '$box/out'"
  check "${FUNCNAME[0]}" "[ ! -e '$LOG/argv' ]"
}

for t in the_key_script_never_prints_the_key init_refuses_when_a_key_exists \
  rotate_keeps_the_old_key_for_one_release rotate_without_a_key_is_refused; do
  this_failed=0
  "$t"
  [ "$this_failed" = 0 ] && echo "ok $t"
  rm -rf "$box"
done
exit "$failed"
