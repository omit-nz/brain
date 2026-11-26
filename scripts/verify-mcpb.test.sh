#!/usr/bin/env bash
# Tests for verify-mcpb.sh: a pack made and signed as the release job makes it passes, and each
# change a packer could slip in is refused, as is a pack Claude Desktop's zip reader would refuse.
# Runs on Linux and macOS; needs zip, unzip, jq, openssl, node and mcpb/tool's packages
# (npm ci --ignore-scripts --prefix mcpb/tool).
#
# Usage: verify-mcpb.test.sh
#
# The tests are called from a list, which the linter reads as unused functions.
# shellcheck disable=SC2317,SC2329
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
version=9.9.9
failed=0

# An extension folder as the release job's Pack step lays it out, and a binary to compare with.
setup() {
  box=$(mktemp -d)
  mkdir -p "$box/ext/server"
  printf 'brain binary' >"$box/brain"
  cp "$box/brain" "$box/ext/server/brain"
  chmod 755 "$box/ext/server/brain"
  cp "$here/../mcpb/icon.png" "$box/ext/"
  jq --arg v "$version" '.version = $v' "$here/../mcpb/manifest.json" >"$box/ext/manifest.json"
}

# A throwaway self-signed certificate: the signature's form is under test, not its chain.
cert() {
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$box/key.pem" -out "$box/cert.pem" \
    -subj /CN=brain-test -days 1 2>/dev/null
}

zipped() {
  (cd "$box/ext" && zip -qr "$box/brain.mcpb" .)
}

# Packed and signed as the release job does it.
pack() {
  zipped
  cert
  bash "$here/sign-mcpb.sh" "$box/brain.mcpb" "$box/cert.pem" "$box/key.pem"
}

verify() {
  bash "$here/verify-mcpb.sh" "$box/brain.mcpb" "$box/brain" "$version" >"$box/out" 2>&1
}

expect_pass() {
  if ! verify; then
    echo "FAIL $1: refused"
    cat "$box/out"
    failed=1
  else
    echo "ok $1"
  fi
}

expect_refused() {
  if verify; then
    echo "FAIL $1: passed"
    failed=1
  else
    echo "ok $1"
  fi
}

a_pack_as_the_release_makes_it_passes() {
  setup
  pack
  expect_pass "${FUNCNAME[0]}"
}

an_unsigned_pack_is_refused() {
  setup
  zipped
  expect_refused "${FUNCNAME[0]}"
}

# What mcpb sign alone makes (v0.2.0's .mcpb): the block after the zip, undeclared, which Claude
# Desktop refuses with "Invalid comment length".
a_signature_after_the_zip_is_refused() {
  setup
  zipped
  cert
  "$here/../mcpb/tool/node_modules/.bin/mcpb" sign "$box/brain.mcpb" -c "$box/cert.pem" -k "$box/key.pem" >/dev/null
  expect_refused "${FUNCNAME[0]}"
}

# A block whose own length field disagrees with it: not one signature, whatever its ends say.
a_comment_that_is_not_one_signature_block_is_refused() {
  setup
  pack
  local at
  at=$(grep -abo MCPB_SIG_V1 "$box/brain.mcpb" | tail -1 | cut -d: -f1)
  printf '\xff\xff\x00\x00' | dd of="$box/brain.mcpb" bs=1 seek=$((at + 11)) conv=notrunc 2>/dev/null
  expect_refused "${FUNCNAME[0]}"
}

a_repointed_command_is_refused() {
  setup
  jq '.server.mcp_config.command = "/bin/sh"' "$box/ext/manifest.json" >"$box/m"
  mv "$box/m" "$box/ext/manifest.json"
  pack
  expect_refused "${FUNCNAME[0]}"
}

an_extra_file_is_refused() {
  setup
  printf 'x' >"$box/ext/server/helper"
  pack
  expect_refused "${FUNCNAME[0]}"
}

another_binary_is_refused() {
  setup
  printf 'other binary' >"$box/ext/server/brain"
  pack
  expect_refused "${FUNCNAME[0]}"
}

the_wrong_version_is_refused() {
  setup
  jq '.version = "1.0.0"' "$box/ext/manifest.json" >"$box/m"
  mv "$box/m" "$box/ext/manifest.json"
  pack
  expect_refused "${FUNCNAME[0]}"
}

a_server_that_cannot_run_is_refused() {
  setup
  chmod 644 "$box/ext/server/brain"
  pack
  expect_refused "${FUNCNAME[0]}"
}

for t in a_pack_as_the_release_makes_it_passes an_unsigned_pack_is_refused a_signature_after_the_zip_is_refused \
  a_comment_that_is_not_one_signature_block_is_refused \
  a_repointed_command_is_refused \
  an_extra_file_is_refused another_binary_is_refused the_wrong_version_is_refused \
  a_server_that_cannot_run_is_refused; do
  "$t"
done
exit "$failed"
