#!/usr/bin/env bash
# Tests for release-key.sh, with zipsign, op and gh stubbed on PATH. The stubs keep a small
# 1Password (item titles in $LOG/items) and a small repo (secret names in $LOG/secrets), record
# every argv and every stdin, and echo what they are given back on stdout and stderr, as a real
# tool's error might, so a test can say where the private key went.
# Runs on Linux and macOS.
#
# Usage: release-key.test.sh
#
# Each check is a string evaluated after the run, and the tests are called from a list, which
# the linter reads as unused functions (SC2317 or SC2329 by version), unexpanded quotes and an
# unused $status.
# shellcheck disable=SC2016,SC2034,SC2317,SC2329
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
item="brain release signing key"
failed=0

# A sandbox repo holding a copy of the script and the placeholder key, plus the stubs.
setup() {
  box=$(mktemp -d)
  mkdir -p "$box/repo/scripts" "$box/repo/crates/brain/src/keys" "$box/bin" "$box/log" "$box/tmp"
  cp "$here/release-key.sh" "$box/repo/scripts/"
  head -c 32 /dev/zero >"$box/repo/crates/brain/src/keys/release.pub"
  : >"$box/log/items"
  : >"$box/log/secrets"

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
in="$LOG/op-stdin.$n"
: >"$in"
case "$1 $2" in
  "item list")
    jq -R -s 'split("\n") | map(select(. != "") | {title: .})' "$LOG/items" ;;
  "item create")
    cat >"$in"
    sed -n 's/.*"title":"\([^"]*\)".*/\1/p' "$in" >>"$LOG/items"
    echo "{\"created\": $(cat "$in")}"
    echo "op: created $(cat "$in")" >&2 ;;
  "item edit")
    from=$3; to=
    while [ $# -gt 0 ]; do [ "$1" = --title ] && to=$2; shift; done
    grep -qxF "$from" "$LOG/items" || exit 1
    { grep -vxF "$from" "$LOG/items" || true; echo "$to"; } >"$LOG/items.new"
    mv "$LOG/items.new" "$LOG/items" ;;
  "read op:"*)
    title=${2#op://*/}; title=${title%/password}
    [ "$(grep -cxF "$title" "$LOG/items")" = 1 ] || exit 1
    printf '%s' "$OLD_KEY_B64" ;;
  *) exit 2 ;;
esac
EOF

  cat >"$box/bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh $*" >>"$LOG/argv"
case "$1 $2" in
  "secret list") sed 's/$/\t2026-11-25T00:00:00Z/' "$LOG/secrets" ;;
  "secret set")
    n=$(ls "$LOG" | grep -c '^gh-stdin' || true)
    cat >"$LOG/gh-stdin.$n"
    echo "$3" >"$LOG/gh-name.$n"
    if [ "$3" = "${GH_FAIL:-}" ]; then echo "gh: could not set $3 to $(cat "$LOG/gh-stdin.$n")" >&2; exit 1; fi
    echo "$3" >>"$LOG/secrets"
    echo "Set Actions secret $3 to $(cat "$LOG/gh-stdin.$n")" ;;
  *) exit 2 ;;
esac
EOF
  chmod +x "$box/bin/"*
  export LOG="$box/log"
  OLD_KEY_B64=
  GH_FAIL=
}

run_script() {
  set +e
  PATH="$box/bin:$PATH" TMPDIR="$box/tmp" OLD_KEY_B64="$OLD_KEY_B64" GH_FAIL="$GH_FAIL" \
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

key_n() { printf 'SENTINEL-PRIVATE-KEY-%02d-%040d' "$1" 0; }
pub_n() { printf 'PUBLIC-KEY-%02d-%018d' "$1" 0; }
b64() { printf '%s' "$1" | base64 | tr -d '\n'; }

# Neither the key nor its base64 form appears in any argv or in the script's own output.
never_printed() {
  local name=$1 key=$2
  check "$name" "! grep -qF -e '$key' -e '$(b64 "$key")' '$LOG/argv'"
  check "$name" "! grep -qF -e '$key' -e '$(b64 "$key")' '$box/out'"
}

# The stdin gh was given for its last `secret set <name>`.
set_stdin() {
  local f last=
  for f in "$LOG"/gh-name.*; do
    [ -e "$f" ] && [ "$(cat "$f")" = "$1" ] && last=${f/gh-name/gh-stdin}
  done
  [ -n "$last" ] && cat "$last"
}

sets_of() { grep -c "gh secret set $1 " "$LOG/argv" || true; }
clean_tmp() { check "$1" "[ -z \"\$(ls -A '$box/tmp')\" ]"; }

the_key_script_never_prints_the_key() {
  local t=${FUNCNAME[0]}
  setup
  run_script init
  check "$t" '[ "$status" = 0 ]'
  never_printed "$t" "$(key_n 1)"
  check "$t" "grep -rqF '$(b64 "$(key_n 1)")' '$LOG'/op-stdin.*"
  check "$t" "[ \"\$(set_stdin RELEASE_SIGNING_KEY)\" = '$(b64 "$(key_n 1)")' ]"
  check "$t" "grep -q 'gh secret set RELEASE_SIGNING_KEY --repo omit-nz/brain' '$LOG/argv'"
  check "$t" "[ \"\$(cat '$LOG/items')\" = '$item' ]"
  check "$t" "[ \"\$(cat '$box/repo/crates/brain/src/keys/release.pub')\" = '$(pub_n 1)' ]"
  clean_tmp "$t"
}

init_refuses_when_a_key_exists() {
  local t=${FUNCNAME[0]}
  setup
  pub_n 7 >"$box/repo/crates/brain/src/keys/release.pub"
  run_script init
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'rotate' '$box/out'"
  check "$t" "[ ! -e '$LOG/argv' ]"
}

init_refuses_when_the_vault_already_holds_a_key() {
  local t=${FUNCNAME[0]}
  setup
  echo "$item" >"$LOG/items"
  run_script init
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q '1Password' '$box/out'"
  check "$t" "! grep -q -e 'secret set' -e 'item create' -e zipsign '$LOG/argv'"
}

init_refuses_when_the_repo_secret_exists() {
  local t=${FUNCNAME[0]}
  setup
  echo RELEASE_SIGNING_KEY >"$LOG/secrets"
  run_script init
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'RELEASE_SIGNING_KEY' '$box/out'"
  check "$t" "! grep -q -e 'secret set' -e 'item create' -e zipsign '$LOG/argv'"
}

init_cleans_up_and_stays_quiet_when_gh_fails() {
  local t=${FUNCNAME[0]}
  setup
  GH_FAIL=RELEASE_SIGNING_KEY
  run_script init
  check "$t" '[ "$status" != 0 ]'
  never_printed "$t" "$(key_n 1)"
  check "$t" "[ \"\$(od -An -v -tx1 '$box/repo/crates/brain/src/keys/release.pub' | tr -d ' \n')\" = \"\$(printf '%064d' 0)\" ]"
  clean_tmp "$t"
  GH_FAIL=
  run_script init
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'pending' '$box/out'"
}

rotate_keeps_the_old_key_for_one_release() {
  local t=${FUNCNAME[0]}
  setup
  OLD_KEY_B64=$(b64 "$(key_n 0)")
  echo "$item" >"$LOG/items"
  echo RELEASE_SIGNING_KEY >"$LOG/secrets"
  pub_n 0 >"$box/repo/crates/brain/src/keys/release.pub"
  run_script rotate
  check "$t" '[ "$status" = 0 ]'
  never_printed "$t" "$(key_n 0)"
  never_printed "$t" "$(key_n 1)"
  check "$t" "[ \"\$(set_stdin RELEASE_SIGNING_KEY_PREVIOUS)\" = '$OLD_KEY_B64' ]"
  check "$t" "[ \"\$(set_stdin RELEASE_SIGNING_KEY)\" = '$(b64 "$(key_n 1)")' ]"
  check "$t" "grep -qxF '$item' '$LOG/items'"
  check "$t" "grep -q '^$item (retired ' '$LOG/items'"
  check "$t" "! grep -q 'pending' '$LOG/items'"
  check "$t" "[ \"\$(cat '$box/repo/crates/brain/src/keys/release.pub')\" = '$(pub_n 1)' ]"
  clean_tmp "$t"
}

rotate_without_a_key_is_refused() {
  local t=${FUNCNAME[0]}
  setup
  run_script rotate
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'init' '$box/out'"
  check "$t" "[ ! -e '$LOG/argv' ]"
}

rotate_without_a_pub_file_is_refused() {
  local t=${FUNCNAME[0]}
  setup
  rm "$box/repo/crates/brain/src/keys/release.pub"
  run_script rotate
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'release.pub' '$box/out'"
  check "$t" "[ ! -e '$LOG/argv' ]"
}

rotate_needs_exactly_one_key_in_the_vault() {
  local t=${FUNCNAME[0]} items
  for items in "" "$item"$'\n'"$item"; do
    setup
    printf '%s\n' "$items" >"$LOG/items"
    pub_n 0 >"$box/repo/crates/brain/src/keys/release.pub"
    run_script rotate
    check "$t" '[ "$status" != 0 ]'
    check "$t" "! grep -q -e 'secret set' -e 'item create' -e 'item edit' '$LOG/argv'"
    rm -rf "$box"
  done
  setup
}

a_rotation_that_stops_part_way_keeps_the_old_key_until_repaired() {
  local t=${FUNCNAME[0]}
  setup
  OLD_KEY_B64=$(b64 "$(key_n 0)")
  echo "$item" >"$LOG/items"
  pub_n 0 >"$box/repo/crates/brain/src/keys/release.pub"
  GH_FAIL=RELEASE_SIGNING_KEY
  run_script rotate
  check "$t" '[ "$status" != 0 ]'
  never_printed "$t" "$(key_n 1)"
  check "$t" "[ \"\$(set_stdin RELEASE_SIGNING_KEY_PREVIOUS)\" = '$OLD_KEY_B64' ]"
  check "$t" "[ \"\$(cat '$box/repo/crates/brain/src/keys/release.pub')\" = '$(pub_n 0)' ]"
  clean_tmp "$t"
  GH_FAIL=
  run_script rotate
  check "$t" '[ "$status" != 0 ]'
  check "$t" "grep -q 'pending' '$box/out'"
  check "$t" "[ \"\$(sets_of RELEASE_SIGNING_KEY_PREVIOUS)\" = 1 ]"
  # The repair docs/release.md gives: delete the pending item, then run again.
  { grep -v 'pending' "$LOG/items" || true; } >"$LOG/items.new"
  mv "$LOG/items.new" "$LOG/items"
  run_script rotate
  check "$t" '[ "$status" = 0 ]'
  check "$t" "[ \"\$(set_stdin RELEASE_SIGNING_KEY_PREVIOUS)\" = '$OLD_KEY_B64' ]"
  check "$t" "[ \"\$(set_stdin RELEASE_SIGNING_KEY)\" = '$(b64 "$(key_n 2)")' ]"
}

for t in the_key_script_never_prints_the_key init_refuses_when_a_key_exists \
  init_refuses_when_the_vault_already_holds_a_key init_refuses_when_the_repo_secret_exists \
  init_cleans_up_and_stays_quiet_when_gh_fails rotate_keeps_the_old_key_for_one_release \
  rotate_without_a_key_is_refused rotate_without_a_pub_file_is_refused \
  rotate_needs_exactly_one_key_in_the_vault \
  a_rotation_that_stops_part_way_keeps_the_old_key_until_repaired; do
  this_failed=0
  "$t"
  [ "$this_failed" = 0 ] && echo "ok $t"
  rm -rf "$box"
done
exit "$failed"
